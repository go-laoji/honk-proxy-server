//! Process-shared facts; each generation retains its own required profile fingerprints.
use hps_domain::{HealthTransport, HealthVerdict, IpFamily, ProbePhase, ProbeResult};
use std::{
    collections::HashMap,
    sync::RwLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Default)]
pub struct HealthPolicy {
    pub enabled: bool,
    pub profiles: Vec<(String, HealthTransport, IpFamily)>,
    pub max_staleness_secs: u64,
    pub traffic_failure_threshold: u32,
}
struct Fact {
    result: ProbeResult,
    fresh_until: Instant,
}
pub use hps_domain::TrafficFeedback;
pub struct HealthRegistry {
    facts: RwLock<HashMap<(String, String), Fact>>,
    failures: RwLock<HashMap<String, (u32, Instant)>>,
    events: tokio::sync::broadcast::Sender<TrafficFeedback>,
    last_traffic: RwLock<HashMap<String, TrafficFeedback>>,
}
impl Default for HealthRegistry {
    fn default() -> Self {
        Self {
            facts: Default::default(),
            failures: Default::default(),
            events: tokio::sync::broadcast::channel(1024).0,
            last_traffic: Default::default(),
        }
    }
}
impl HealthRegistry {
    pub fn last_traffic(&self, identity: &str) -> Option<TrafficFeedback> {
        self.last_traffic.read().unwrap().get(identity).cloned()
    }
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TrafficFeedback> {
        self.events.subscribe()
    }
    pub fn publish(&self, result: ProbeResult, maximum_age: u64) {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let timestamp = Duration::from_secs(result.checked_at.max(0) as u64);
        let lifetime = wall.checked_sub(timestamp).map_or(Duration::ZERO, |age| {
            Duration::from_secs(maximum_age).saturating_sub(age)
        });
        if result.verdict == HealthVerdict::Healthy {
            // A delayed completion cannot clear a later real connection failure.
            let mut failures = self.failures.write().unwrap();
            if failures
                .get(&result.identity)
                .is_none_or(|(_, at)| at.elapsed() >= Duration::from_millis(result.latency_ms))
            {
                failures.remove(&result.identity);
            }
        }
        let mut facts = self.facts.write().unwrap();
        if facts.len() >= 400000 {
            facts.retain(|_, fact| fact.fresh_until > Instant::now());
        }
        if facts.len() < 400000
            || facts.contains_key(&(result.identity.clone(), result.profile_fingerprint.clone()))
        {
            facts.insert(
                (result.identity.clone(), result.profile_fingerprint.clone()),
                Fact {
                    result,
                    fresh_until: Instant::now() + lifetime,
                },
            );
        }
    }
    pub fn latest(
        &self,
        identity: &str,
        fingerprint: &str,
        maximum_age: u64,
    ) -> Option<ProbeResult> {
        let facts = self.facts.read().unwrap();
        let fact = facts.get(&(identity.into(), fingerprint.into()))?;
        let mut result = fact.result.clone();
        if result.verdict == HealthVerdict::Healthy
            && (fact.fresh_until <= Instant::now()
                || unix_now().saturating_sub(result.checked_at) > maximum_age as i64
                || result.checked_at > unix_now())
        {
            result.verdict = HealthVerdict::Unknown;
            result.error_code = Some("health_stale".into());
        }
        if result.verdict == HealthVerdict::Healthy
            && self
                .failures
                .read()
                .unwrap()
                .get(identity)
                .is_some_and(|(n, _)| *n == u32::MAX)
        {
            result.verdict = HealthVerdict::Degraded;
            result.error_code = Some("real_traffic_failure".into());
        }
        Some(result)
    }
    pub fn usable(&self, identity: &str, policy: &HealthPolicy, family: Option<IpFamily>) -> bool {
        !policy.enabled
            || policy.profiles.iter().any(|(fp, transport, f)| {
                *transport == HealthTransport::Tcp
                    && family.is_none_or(|family| family == *f)
                    && self
                        .latest(identity, fp, policy.max_staleness_secs)
                        .is_some_and(|r| r.verdict == HealthVerdict::Healthy)
            })
    }
    pub fn traffic(
        &self,
        node_id: &str,
        identity: &str,
        phase: ProbePhase,
        code: &str,
        global_failure: bool,
        threshold: u32,
    ) {
        let mut invalidated = false;
        if global_failure {
            let mut failures = self.failures.write().unwrap();
            if failures.len() >= 100000 {
                failures.retain(|_, (_, at)| at.elapsed() < Duration::from_secs(86400));
            }
            if failures.len() < 100000 || failures.contains_key(identity) {
                let entry = failures
                    .entry(identity.into())
                    .or_insert((0, Instant::now()));
                entry.0 = entry.0.saturating_add(1);
                entry.1 = Instant::now();
                if entry.0 >= threshold.max(1) {
                    entry.0 = u32::MAX;
                    invalidated = true;
                }
            }
        }
        let event = TrafficFeedback {
            node_id: node_id.into(),
            identity: identity.into(),
            phase,
            error_code: code.into(),
            invalidated,
            observed_at: unix_now(),
        };
        let mut traffic = self.last_traffic.write().unwrap();
        if traffic.len() >= 100000 {
            traffic.retain(|_, value| value.observed_at >= unix_now() - 86400);
        }
        if traffic.len() < 100000 || traffic.contains_key(identity) {
            traffic.insert(identity.into(), event.clone());
        }
        drop(traffic);
        let _ = self.events.send(event);
    }
}
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    fn result(identity: &str, fp: &str, family: IpFamily) -> ProbeResult {
        ProbeResult {
            node_id: "lease:1".into(),
            identity: identity.into(),
            profile_fingerprint: fp.into(),
            transport: HealthTransport::Tcp,
            family,
            verdict: HealthVerdict::Healthy,
            phase: ProbePhase::Complete,
            error_code: None,
            endpoint_reachable: true,
            authenticated: Some(true),
            tunnel_established: true,
            http_status: Some(200),
            target_ok: Some(true),
            latency_ms: 0,
            checked_at: unix_now(),
        }
    }
    #[test]
    fn credentials_policy_family_age_and_real_failures_gate_one_fact_source() {
        let registry = HealthRegistry::default();
        let policy = HealthPolicy {
            enabled: true,
            profiles: vec![("p4".into(), HealthTransport::Tcp, IpFamily::Ipv4)],
            max_staleness_secs: 60,
            traffic_failure_threshold: 2,
        };
        assert!(!registry.usable("a", &policy, None));
        registry.publish(result("a", "p4", IpFamily::Ipv4), 60);
        assert!(registry.usable("a", &policy, Some(IpFamily::Ipv4)));
        assert!(!registry.usable("b", &policy, None));
        assert!(!registry.usable("a", &policy, Some(IpFamily::Ipv6)));
        let changed = HealthPolicy {
            profiles: vec![("p-new".into(), HealthTransport::Tcp, IpFamily::Ipv4)],
            ..policy.clone()
        };
        assert!(!registry.usable("a", &changed, None));
        registry.traffic(
            "lease:1",
            "a",
            ProbePhase::TargetHttp,
            "target_rejected",
            false,
            2,
        );
        assert!(registry.usable("a", &policy, None));
        for _ in 0..2 {
            registry.traffic(
                "lease:1",
                "a",
                ProbePhase::Authentication,
                "upstream_authentication_failed",
                true,
                2,
            );
        }
        assert!(!registry.usable("a", &policy, None));
        registry.publish(result("a", "p4", IpFamily::Ipv4), 60);
        assert!(registry.usable("a", &policy, None));
        let mut stale = result("a", "p4", IpFamily::Ipv4);
        stale.checked_at -= 61;
        registry.publish(stale, 60);
        assert!(!registry.usable("a", &policy, None));
        let mut future = result("a", "p4", IpFamily::Ipv4);
        future.checked_at += 100;
        registry.publish(future, 60);
        assert!(!registry.usable("a", &policy, None));
    }
}
