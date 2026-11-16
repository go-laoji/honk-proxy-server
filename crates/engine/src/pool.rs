//! Atomically published dynamic leases. Existing connected sockets retain their owner.
use crate::{ProxyError, dial::Upstream};
use hps_config::UpstreamConfig;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Notify, watch};

const FAILURE_BATCH_SIZE: usize = 128;

#[derive(Clone)]
pub struct FailedLease {
    pub id: i64,
    pub provider_id: String,
    pub fingerprint: String,
    pub(crate) identity: String,
}
#[derive(Debug, Clone)]
pub struct PoolNode {
    pub provider_id: String,
    pub fingerprint: String,
    pub upstream: UpstreamConfig,
    pub min_remaining_secs: u64,
    pub location: Option<hps_domain::NodeLocation>,
}
#[derive(Default)]
pub struct DynamicPool {
    nodes: RwLock<Arc<Vec<Arc<Upstream>>>>,
    clock: Mutex<LeaseClock>,
    track_failures: AtomicBool,
    failures: Mutex<HashMap<i64, FailedLease>>,
    failure_ready: Notify,
    changed: watch::Sender<()>,
}
struct LeaseClock {
    wall: Duration,
    instant: Instant,
}
impl Default for LeaseClock {
    fn default() -> Self {
        let instant = Instant::now();
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self { wall, instant }
    }
}
impl LeaseClock {
    fn now(&mut self) -> Duration {
        // Sample the monotonic reference first, keeping conversion conservative.
        let instant = Instant::now();
        let observed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let logical = self
            .wall
            .saturating_add(instant.saturating_duration_since(self.instant));
        if observed > logical {
            self.wall = observed;
            self.instant = instant;
            observed
        } else {
            logical
        }
    }
}
impl DynamicPool {
    pub fn enable_failure_reporting(&self) {
        self.track_failures.store(true, Ordering::Release);
    }
    pub async fn failure_ready(&self) {
        self.failure_ready.notified().await;
    }
    pub fn failed_leases(&self) -> Vec<FailedLease> {
        self.failures
            .lock()
            .unwrap()
            .values()
            .take(FAILURE_BATCH_SIZE)
            .cloned()
            .collect()
    }
    pub fn complete_failures(&self, ids: Vec<i64>) {
        let mut failures = self.failures.lock().unwrap();
        for id in ids {
            failures.remove(&id);
        }
        if failures.len() >= FAILURE_BATCH_SIZE {
            self.failure_ready.notify_one();
        }
    }
    pub(crate) fn report_failure(&self, lease: &FailedLease) -> bool {
        if !self.track_failures.load(Ordering::Acquire) {
            return false;
        }
        let mut failures = self.failures.lock().unwrap();
        if failures.contains_key(&lease.id) {
            return true;
        }
        if failures.len() >= 100000 {
            return false;
        }
        failures.insert(lease.id, lease.clone());
        self.failure_ready.notify_one();
        true
    }
    pub(crate) fn is_failed(&self, upstream: &Upstream) -> bool {
        upstream.lease_id().is_some_and(|id| self.failed_lease(id))
    }
    pub fn failed_lease(&self, id: i64) -> bool {
        self.failures.lock().unwrap().contains_key(&id)
    }
    pub(crate) fn lease_is_current(&self, upstream: &Upstream) -> bool {
        let Some(id) = upstream.lease_id() else {
            return true;
        };
        let pending = self.failures.lock().unwrap();
        !pending.contains_key(&id)
            && self
                .nodes
                .read()
                .unwrap()
                .iter()
                .any(|node| node.config.id == upstream.config.id)
    }
    pub fn notify_change(&self) {
        self.changed.send_replace(());
    }
    pub(crate) fn changes(&self) -> watch::Receiver<()> {
        self.changed.subscribe()
    }
    pub fn logical_now(&self) -> u64 {
        self.clock.lock().unwrap().now().as_secs()
    }
    pub(crate) fn snapshot(&self) -> Arc<Vec<Arc<Upstream>>> {
        self.nodes.read().unwrap().clone()
    }
    /// Refresh derived metadata while retaining prepared TLS, identity and lease deadlines.
    pub fn reclassify(&self, mut locate: impl FnMut(&str) -> Option<hps_domain::NodeLocation>) {
        loop {
            let previous = self.snapshot();
            let nodes = previous
                .iter()
                .map(|upstream| {
                    let location = locate(&upstream.config.host);
                    if location == upstream.location {
                        return upstream.clone();
                    }
                    let mut updated = (**upstream).clone();
                    updated.location = location;
                    Arc::new(updated)
                })
                .collect();
            let mut current = self.nodes.write().unwrap();
            if Arc::ptr_eq(&previous, &current) {
                *current = Arc::new(nodes);
                break;
            }
        }
    }
    /// Prepare before replacing. Re-publishing a lease never extends its monotonic deadline.
    pub fn replace(&self, nodes: Vec<PoolNode>) -> Result<(), ProxyError> {
        if nodes.len() > 100000 {
            return Err(ProxyError::Protocol);
        }
        let previous = self.snapshot();
        let deadlines: HashMap<_, _> = previous
            .iter()
            .map(|n| (n.config.id.clone(), n.deadline))
            .collect();
        let instant = Instant::now();
        let wall = self.clock.lock().unwrap().now();
        let prepared = nodes
            .into_iter()
            .map(|node| {
                let mut upstream = Upstream::prepare(node.upstream)?;
                upstream.source = Some((node.provider_id, node.fingerprint));
                upstream.minimum_lifetime = node.min_remaining_secs;
                upstream.location = node.location;
                upstream.deadline = upstream.config.expires_at_unix.map(|expiry| {
                    instant
                        + Duration::from_secs(expiry)
                            .saturating_sub(wall)
                            .min(Duration::from_secs(31_536_000))
                });
                if let Some(Some(old)) = deadlines.get(&upstream.config.id) {
                    upstream.deadline = upstream.deadline.map(|new| new.min(*old));
                }
                Ok(Arc::new(upstream))
            })
            .collect::<Result<Vec<_>, ProxyError>>()?;
        *self.nodes.write().unwrap() = Arc::new(prepared);
        self.notify_change();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(expiry: u64) -> PoolNode {
        PoolNode {
            provider_id: "vendor".into(),
            fingerprint: "f".into(),
            min_remaining_secs: 0,
            location: None,
            upstream: UpstreamConfig {
                id: "lease-1".into(),
                protocol: hps_config::ProxyProtocol::Http,
                host: "192.0.2.1".into(),
                port: 8080,
                enabled: true,
                credentials: None,
                expires_at_unix: Some(expiry),
                ca_certificate: None,
                tls_server_name: None,
            },
        }
    }
    #[test]
    fn republishing_or_removing_a_lease_never_moves_clock_backwards() {
        let pool = DynamicPool::default();
        let now = pool.logical_now();
        pool.replace(vec![node(now + 100)]).unwrap();
        let first = pool.snapshot()[0].deadline.unwrap();
        pool.replace(vec![node(now + 1000)]).unwrap();
        assert!(pool.snapshot()[0].deadline.unwrap() <= first);
        pool.replace(vec![]).unwrap();
        pool.clock.lock().unwrap().wall = Duration::from_secs(now + 200);
        pool.replace(vec![node(now + 100)]).unwrap();
        assert!(pool.snapshot()[0].deadline.unwrap() <= Instant::now());
    }
    #[test]
    fn fractional_lease_time_is_conservative_and_large_admission_windows_do_not_panic() {
        let pool = Arc::new(DynamicPool::default());
        let now = pool.logical_now();
        {
            let mut clock = pool.clock.lock().unwrap();
            clock.wall = Duration::from_secs(now + 10) + Duration::from_millis(900);
            clock.instant = Instant::now();
        }
        pool.replace(vec![node(now + 11)]).unwrap();
        assert!(
            pool.snapshot()[0]
                .deadline
                .unwrap()
                .saturating_duration_since(Instant::now())
                <= Duration::from_millis(100)
        );
        let mut config = hps_config::ProxyConfig::default();
        config.limits.min_remaining_lifetime_secs = u64::MAX;
        let mut dialer = crate::dial::Dialer::new(&config).unwrap();
        dialer.pool = pool;
        dialer.set_sources(HashMap::from([("vendor".into(), "f".into())]));
        assert_eq!(dialer.eligible_count(), 0);
    }
}
