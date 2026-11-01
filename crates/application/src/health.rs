use super::*;
use hps_domain::{
    HealthOverview, HealthProbeInput, HealthTask, HealthTransport, HealthVerdict, IpFamily,
    NodeHealth, ProbePhase, ProbeResult,
};
use hps_engine::dial::ProbePlan;
use hps_storage::health::{HealthState, ProbeReservation};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};

pub(super) struct HealthRuntime {
    started: AtomicBool,
    active: AtomicUsize,
    bootstrap_blocked: AtomicBool,
    cursor: AtomicUsize,
    states: Mutex<HashMap<(String, String), HealthState>>,
    bootstrap_checked: Mutex<Option<(String, std::time::Instant)>>,
}
impl Default for HealthRuntime {
    fn default() -> Self {
        Self {
            started: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            bootstrap_blocked: AtomicBool::new(true),
            cursor: AtomicUsize::new(0),
            states: Mutex::new(HashMap::new()),
            bootstrap_checked: Mutex::new(None),
        }
    }
}
impl HealthRuntime {
    pub(super) fn publish_generation(&self, changed: bool, publish: impl FnOnce()) {
        let mut checked = self.bootstrap_checked.lock().unwrap();
        if changed {
            self.bootstrap_blocked.store(true, Ordering::Release);
            *checked = None;
        }
        publish();
    }
}
impl Supervisor {
    pub(super) async fn load_health(&self) -> Result<(), ApplicationError> {
        let states = self.store.health_states().await?;
        let current = self.current();
        let mut map = self.health.states.lock().unwrap();
        for state in states {
            current.engine.health_registry().publish(
                state.result.clone(),
                current.config.health.max_staleness_secs,
            );
            map.insert(
                (
                    state.result.identity.clone(),
                    state.result.profile_fingerprint.clone(),
                ),
                state,
            );
        }
        Ok(())
    }
    pub(super) fn start_health_scheduler(self: &Arc<Self>) {
        if self.health.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        let cancel = self.provider_cancel.clone();
        let mut feedback = self.current().engine.health_registry().subscribe();
        self.provider_tasks.spawn(async move {
            let mut timer=tokio::time::interval(Duration::from_millis(250));timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;_=cancel.cancelled()=>break,_=timer.tick()=>{let Some(service)=weak.upgrade() else {break;};let _=service.health_tick().await;},event=feedback.recv()=>{
                    let Some(service)=weak.upgrade() else {break;};
                    match event {Ok(event)=>service.health_feedback(event).await,Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=>{},Err(_)=>break}
                }}
            }
        });
    }
    async fn health_feedback(&self, event: hps_engine::health::TrafficFeedback) {
        if !event.invalidated {
            return;
        }
        // The shared registry already blocked new selections synchronously. Persist the invalidation.
        let current = self.current();
        let profiles = current
            .health_profiles
            .iter()
            .map(|p| p.fingerprint.clone())
            .collect::<HashSet<_>>();
        let mut records = vec![];
        {
            let mut states = self.health.states.lock().unwrap();
            for ((identity, fp), state) in states.iter_mut() {
                if *identity != event.identity || !profiles.contains(fp) {
                    continue;
                }
                state.result.verdict = HealthVerdict::Failed;
                state.result.phase = event.phase;
                state.result.error_code = Some(event.error_code.clone());
                state.result.checked_at = event.observed_at;
                state.result.node_id = event.node_id.clone();
                state.result.target_ok = None;
                state.result.http_status = None;
                state.result.endpoint_reachable = event.phase != ProbePhase::EndpointConnect;
                state.result.authenticated = if event.phase == ProbePhase::Authentication {
                    Some(false)
                } else {
                    None
                };
                state.result.tunnel_established = false;
                state.next_due_at = event.observed_at;
                records.push(state.clone());
            }
        }
        for record in records {
            let _ = self.store.invalidate_health(record).await;
        }
    }
    async fn health_tick(self: &Arc<Self>) -> Result<(), ApplicationError> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let current = self.current();
        if !current.config.health.enabled {
            return Ok(());
        }
        let config = &current.config.health;
        let key = current
            .health_profiles
            .iter()
            .map(|p| p.fingerprint.as_str())
            .collect::<Vec<_>>()
            .join(":");
        let check = self
            .health
            .bootstrap_checked
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|(old, when)| *old != key || when.elapsed() >= Duration::from_secs(5));
        if check {
            let mut bootstrap = false;
            for profile in &current.health_profiles {
                if tokio::time::timeout(
                    Duration::from_millis(config.timeout_ms),
                    profile.bootstrap_ready(),
                )
                .await
                .unwrap_or(false)
                {
                    bootstrap = true;
                    break;
                }
            }
            let mut checked = self.health.bootstrap_checked.lock().unwrap();
            let latest = self.current();
            if current.dns.fingerprint() == latest.dns.fingerprint()
                && current
                    .health_profiles
                    .iter()
                    .map(|p| &p.fingerprint)
                    .eq(latest.health_profiles.iter().map(|p| &p.fingerprint))
            {
                self.health
                    .bootstrap_blocked
                    .store(!bootstrap, Ordering::Release);
                // Scheduling cadence is separate from DNS data. System getaddrinfo
                // answers retain TTL=unknown and actual probes resolve afresh.
                *checked = Some((key, std::time::Instant::now()));
            }
        }
        if self.current().revision != current.revision {
            return Ok(());
        }
        let bootstrap = !self.health.bootstrap_blocked.load(Ordering::Acquire);
        let usage = self.store.health_usage().await?;
        let pending = self
            .store
            .pending_health_runs()
            .await?
            .into_iter()
            .map(|r| (r.identity, r.profile_fingerprint))
            .collect::<HashSet<_>>();
        let plans = current.engine.probe_plans();
        let active = self.health.active.load(Ordering::Acquire);
        if active < config.max_parallel {
            let queued = self
                .store
                .queued_health_runs((config.max_parallel - active) as u32)
                .await?;
            for run in queued {
                let plan = plans.iter().find(|p| {
                    p.identity() == run.identity
                        && (p.config().id == run.node_id || p.config().enabled)
                });
                let profile = current
                    .health_profiles
                    .iter()
                    .find(|p| p.fingerprint == run.profile_fingerprint);
                if let (Some(plan), Some(profile)) = (plan, profile) {
                    if !self.store.claim_health_run(run.id).await? {
                        continue;
                    }
                    self.health.active.fetch_add(1, Ordering::AcqRel);
                    let service = self.clone();
                    let plan = plan.clone();
                    let profile = profile.clone();
                    let settings = config.clone();
                    let cancel = self.provider_cancel.clone();
                    self.provider_tasks.spawn(async move {
                        tokio::select! {biased;_=cancel.cancelled()=>{let _=service.store.discard_health_run(run.id).await;},_=service.run_probe(run.id,plan,profile,settings)=>{}}
                        service.health.active.fetch_sub(1,Ordering::AcqRel);
                    });
                } else {
                    self.store.discard_health_run(run.id).await?;
                }
            }
        }
        if !bootstrap {
            return Ok(());
        }
        let free = config
            .queue_capacity
            .saturating_sub((usage.queued + usage.probing) as usize)
            .min(config.daily_probe_budget.saturating_sub(usage.used) as usize)
            .min(64);
        if free == 0 || plans.is_empty() {
            return Ok(());
        }
        let now = hps_storage::unix_now();
        let mut probes = vec![];
        let mut seen = HashSet::new();
        let start = self.health.cursor.load(Ordering::Relaxed) % plans.len();
        let mut scanned = 0;
        {
            let states = self.health.states.lock().unwrap();
            for offset in 0..plans.len().min(5000) {
                let plan = &plans[(start + offset) % plans.len()];
                scanned += 1;
                if !plan.config().enabled
                    || plan.operation().state.blocking_reason().is_some()
                    || !self.probe_lifetime_ok(plan, config)
                {
                    continue;
                }
                for profile in &current.health_profiles {
                    let key = (plan.identity().to_owned(), profile.fingerprint.clone());
                    if pending.contains(&key)
                        || !seen.insert(key.clone())
                        || states.get(&key).is_some_and(|s| {
                            let policy_wait = if s.result.verdict == HealthVerdict::Healthy {
                                config.interval_secs
                            } else {
                                config
                                    .retry_initial_secs
                                    .saturating_mul(1u64 << s.failures.saturating_sub(1).min(16))
                                    .min(config.retry_max_secs)
                            };
                            s.next_due_at
                                .min(s.result.checked_at.saturating_add(policy_wait as i64))
                                > now
                        })
                    {
                        continue;
                    }
                    probes.push(ProbeReservation {
                        node_id: plan.config().id.clone(),
                        identity: key.0,
                        profile_fingerprint: key.1,
                    });
                    if probes.len() >= free {
                        break;
                    }
                }
                if probes.len() >= free {
                    break;
                }
            }
        }
        self.health.cursor.store(start + scanned, Ordering::Relaxed);
        if !probes.is_empty() {
            let _ = self
                .store
                .begin_health_task(
                    uuid::Uuid::new_v4().to_string(),
                    "automatic".into(),
                    probes,
                    config.queue_capacity,
                    config.daily_probe_budget,
                )
                .await?;
        }
        Ok(())
    }
    fn probe_lifetime_ok(&self, plan: &ProbePlan, settings: &hps_config::HealthConfig) -> bool {
        plan.remaining().is_none_or(|remaining| {
            remaining
                > Duration::from_secs(
                    plan.minimum_lifetime()
                        .saturating_add(settings.min_probe_margin_secs),
                )
                .saturating_add(Duration::from_millis(settings.timeout_ms))
        })
    }
    async fn run_probe(
        &self,
        id: i64,
        plan: ProbePlan,
        profile: hps_health::PreparedProfile,
        settings: hps_config::HealthConfig,
    ) {
        let result = profile.probe(&plan, &settings).await;
        let _operation = self.operation.lock().await;
        let current = self.current();
        if !current.config.health.enabled
            || !current
                .health_profiles
                .iter()
                .any(|p| p.fingerprint == result.profile_fingerprint)
            || !current
                .engine
                .probe_plans()
                .iter()
                .any(|p| p.config().enabled && p.identity() == result.identity)
        {
            let _ = self.store.discard_health_run(id).await;
            return;
        }
        let key = (result.identity.clone(), result.profile_fingerprint.clone());
        let failures = if result.verdict == HealthVerdict::Healthy {
            0
        } else {
            self.health
                .states
                .lock()
                .unwrap()
                .get(&key)
                .map_or(1, |s| s.failures.saturating_add(1))
        };
        let wait = if failures == 0 {
            settings.interval_secs
        } else {
            settings
                .retry_initial_secs
                .saturating_mul(1u64 << failures.saturating_sub(1).min(16))
                .min(settings.retry_max_secs)
        };
        let next = result.checked_at.saturating_add(wait as i64);
        if self
            .store
            .finish_health_run(id, result.clone(), next, failures)
            .await
            .is_ok()
        {
            current
                .engine
                .health_registry()
                .publish(result.clone(), current.config.health.max_staleness_secs);
            let mut states = self.health.states.lock().unwrap();
            if states.len() >= 400000 {
                let identities = current
                    .engine
                    .probe_plans()
                    .into_iter()
                    .map(|p| p.identity().to_owned())
                    .collect::<HashSet<_>>();
                let fingerprints = current
                    .health_profiles
                    .iter()
                    .map(|p| p.fingerprint.clone())
                    .collect::<HashSet<_>>();
                states.retain(|(identity, fp), _| {
                    identities.contains(identity) && fingerprints.contains(fp)
                });
            }
            states.insert(
                key,
                HealthState {
                    result,
                    failures,
                    next_due_at: next,
                },
            );
        }
    }
    pub async fn probe_health(
        self: &Arc<Self>,
        input: HealthProbeInput,
    ) -> Result<HealthTask, ApplicationError> {
        let service = self.clone();
        tokio::spawn(async move {
            let _operation = service.operation.lock().await;
            if let Some(task) = service
                .store
                .health_task_by_request(input.request_id.clone())
                .await?
            {
                return Ok(task);
            }
            let current = service.current();
            if !current.config.health.enabled
                || service.closed.load(Ordering::Acquire)
                || input.node_ids.len() > 1000
            {
                return Err(ApplicationError::Invalid);
            }
            let all = current.engine.probe_plans();
            let ids = input.node_ids.into_iter().collect::<HashSet<_>>();
            if ids
                .iter()
                .any(|id| !all.iter().any(|p| p.config().id == *id))
            {
                return Err(ApplicationError::Invalid);
            }
            let plans = all
                .into_iter()
                .filter(|p| ids.is_empty() || ids.contains(&p.config().id))
                .take(1000)
                .collect::<Vec<_>>();
            let probes = plans
                .iter()
                .flat_map(|p| {
                    current
                        .health_profiles
                        .iter()
                        .map(move |profile| ProbeReservation {
                            node_id: p.config().id.clone(),
                            identity: p.identity().into(),
                            profile_fingerprint: profile.fingerprint.clone(),
                        })
                })
                .collect();
            Ok(service
                .store
                .begin_health_task(
                    input.request_id,
                    "manual".into(),
                    probes,
                    current.config.health.queue_capacity,
                    current.config.health.daily_probe_budget,
                )
                .await?)
        })
        .await
        .map_err(|_| ApplicationError::Worker)?
    }
    pub(super) fn plan_health(
        &self,
        plan: &ProbePlan,
        current: &RuntimeSnapshot,
    ) -> (bool, bool, bool) {
        if !current.config.health.enabled {
            return (true, false, false);
        }
        if current.engine.health_usable(plan.identity()) {
            return (true, false, false);
        }
        let registry = current.engine.health_registry();
        let mut pending = false;
        let mut blocked = false;
        for profile in &current.health_profiles {
            match registry.latest(
                plan.identity(),
                &profile.fingerprint,
                current.config.health.max_staleness_secs,
            ) {
                None => pending = true,
                Some(result) => {
                    pending |= matches!(
                        result.verdict,
                        HealthVerdict::Unknown | HealthVerdict::Queued | HealthVerdict::Probing
                    );
                    blocked |= matches!(
                        result.phase,
                        ProbePhase::Authentication
                            | ProbePhase::UpstreamTls
                            | ProbePhase::TargetResolve
                            | ProbePhase::Local
                    );
                }
            }
        }
        (false, pending, blocked)
    }
    pub(super) async fn health_purchase_block(
        &self,
    ) -> Result<Option<&'static str>, ApplicationError> {
        let current = self.current();
        if !current.config.health.enabled {
            return Ok(None);
        }
        if self.health.bootstrap_blocked.load(Ordering::Acquire) {
            return Ok(Some("health_target_unavailable"));
        }
        let usage = self.store.health_usage().await?;
        if usage.used >= current.config.health.daily_probe_budget {
            return Ok(Some("health_budget_exhausted"));
        }
        Ok(None)
    }
    pub async fn health_task(&self, id: i64) -> Result<HealthTask, ApplicationError> {
        Ok(self.store.health_task(id).await?)
    }
    pub async fn health_overview(
        &self,
        after: Option<String>,
        limit: u32,
    ) -> Result<HealthOverview<hps_config::HealthConfig>, ApplicationError> {
        let current = self.current();
        let mut plans = current.engine.probe_plans();
        plans.sort_by(|a, b| a.config().id.cmp(&b.config().id));
        let total = plans.len();
        let mut selected = plans
            .into_iter()
            .filter(|p| after.as_ref().is_none_or(|after| p.config().id > *after))
            .take(limit.clamp(1, 200) as usize + 1)
            .collect::<Vec<_>>();
        let next_after = if selected.len() > limit.clamp(1, 200) as usize {
            selected.pop();
            selected.last().map(|p| p.config().id.clone())
        } else {
            None
        };
        let registry = current.engine.health_registry();
        let pending = self.store.pending_health_runs().await?;
        let mut nodes = vec![];
        for plan in selected {
            let mut results = vec![];
            for transport in [HealthTransport::Tcp, HealthTransport::Udp] {
                for family in [IpFamily::Ipv4, IpFamily::Ipv6] {
                    let profile = current
                        .health_profiles
                        .iter()
                        .find(|p| p.profile.transport == transport && p.profile.family == family);
                    let mut result = profile
                        .and_then(|p| {
                            registry.latest(
                                plan.identity(),
                                &p.fingerprint,
                                current.config.health.max_staleness_secs,
                            )
                        })
                        .unwrap_or_else(|| {
                            placeholder(
                                &plan,
                                transport,
                                family,
                                profile.map(|p| p.fingerprint.as_str()),
                            )
                        });
                    result.node_id = plan.config().id.clone();
                    if let Some(run) = pending.iter().find(|r| {
                        r.identity == plan.identity()
                            && r.profile_fingerprint == result.profile_fingerprint
                    }) {
                        // Keep valid previous evidence during a periodic recheck.
                        if result.verdict != HealthVerdict::Healthy {
                            result.verdict = if run.state == "probing" {
                                HealthVerdict::Probing
                            } else {
                                HealthVerdict::Queued
                            };
                        }
                    }
                    if transport == HealthTransport::Tcp
                        && result.verdict == HealthVerdict::Unknown
                        && !self.probe_lifetime_ok(&plan, &current.config.health)
                    {
                        result.verdict = HealthVerdict::Skipped;
                        result.error_code = Some("lease_margin_insufficient".into());
                    }
                    if transport == HealthTransport::Tcp
                        && plan.remaining().is_some_and(|n| n.is_zero())
                    {
                        result.verdict = HealthVerdict::Expired;
                        result.error_code = Some("lease_expired".into());
                    }
                    results.push(result);
                }
            }
            nodes.push(NodeHealth {
                node_id: plan.config().id.clone(),
                provider_id: plan.provider_id().map(str::to_owned),
                protocol: format!("{:?}", plan.config().protocol).to_lowercase(),
                host: plan.config().host.clone(),
                port: plan.config().port,
                enabled: plan.config().enabled,
                remaining_secs: plan.remaining().map(|r| r.as_secs()),
                eligible: plan.config().enabled
                    && plan.operation().state.blocking_reason().is_none()
                    && plan
                        .remaining()
                        .is_none_or(|r| r > Duration::from_secs(plan.minimum_lifetime()))
                    && current.engine.health_usable(plan.identity()),
                last_traffic: registry.last_traffic(plan.identity()),
                results,
            });
        }
        let usage = self.store.health_usage().await?;
        Ok(HealthOverview {
            running_revision: current.revision,
            configuration: current.config.health.clone(),
            nodes,
            next_after,
            total_nodes: total,
            queued: usage.queued,
            probing: usage.probing,
            day_started_at: usage.day * 86400,
            resets_at: (usage.day + 1) * 86400,
            used_probes: usage.used,
            budget: current.config.health.daily_probe_budget,
            tasks: self.store.health_tasks().await?,
        })
    }
}
fn placeholder(
    plan: &ProbePlan,
    transport: HealthTransport,
    family: IpFamily,
    fp: Option<&str>,
) -> ProbeResult {
    ProbeResult {
        node_id: plan.config().id.clone(),
        identity: plan.identity().into(),
        profile_fingerprint: fp.unwrap_or_default().into(),
        transport,
        family,
        verdict: if transport == HealthTransport::Udp {
            HealthVerdict::Unsupported
        } else {
            HealthVerdict::Unknown
        },
        phase: if transport == HealthTransport::Udp {
            ProbePhase::Unsupported
        } else {
            ProbePhase::Admission
        },
        error_code: Some(
            if transport == HealthTransport::Udp {
                "udp_not_compiled"
            } else if fp.is_none() {
                "profile_disabled"
            } else {
                "not_probed"
            }
            .into(),
        ),
        endpoint_reachable: false,
        authenticated: None,
        tunnel_established: false,
        http_status: None,
        target_ok: None,
        latency_ms: 0,
        checked_at: 0,
    }
}
impl Supervisor {
    pub async fn edit_health(
        self: &Arc<Self>,
        input: hps_config::HealthSave,
    ) -> Result<hps_domain::ConfigActivationResult, ApplicationError> {
        let current = self.current();
        let workspace = self.store.workspace().await?;
        if current.revision != input.expected_revision
            || workspace.desired.revision.id != input.expected_revision
        {
            return Err(StorageError::Conflict.into());
        }
        let mut config = current.base_config.clone();
        config.health = input.configuration;
        let revision = self
            .create_revision(ConfigInput {
                toml: toml::to_string(&config).map_err(|_| ApplicationError::Invalid)?,
                expected_revision: input.expected_revision,
                expected_draft_version: None,
                note: "health policy".into(),
                use_draft_credentials: false,
            })
            .await?;
        self.activate(hps_domain::ConfigActivate {
            revision: revision.id,
            expected_revision: input.expected_revision,
            defer_until_restart: false,
        })
        .await
    }
}

#[cfg(test)]
mod dns_cadence_tests {
    use super::*;
    #[tokio::test]
    async fn bootstrap_cadence_does_not_invent_dns_ttl_and_changes_recheck_before_buying() {
        let mut config = DaemonConfig::default();
        config.health.profiles[0].url = "http://localhost:18081/health".into();
        let service = Supervisor::boot(
            Store::open(None, config).unwrap(),
            StartupOverrides::default(),
        )
        .await
        .unwrap();
        assert!(service.health.bootstrap_blocked.load(Ordering::Acquire));
        for _ in 0..8 {
            service.health_tick().await.unwrap();
        }
        assert!(!service.health.bootstrap_blocked.load(Ordering::Acquire));
        let view = service.dns_overview(None, 50).unwrap();
        assert_eq!(view.cache.misses, 1);
        assert_eq!(view.cache.total, 0);
        let result = service
            .current()
            .dns
            .diagnose(hps_dns::LookupInput {
                name: "localhost".into(),
                query_type: "A".into(),
                purpose: hps_net::ResolvePurpose::Health,
            })
            .await
            .unwrap();
        assert_eq!(result.remaining_ttl, None);
        assert_eq!(service.dns_overview(None, 50).unwrap().cache.misses, 2);
        let mut policy = view.configuration;
        policy.timeout_ms = 3000;
        service
            .edit_dns(hps_config::DnsSave {
                configuration: policy,
                expected_revision: 1,
            })
            .await
            .unwrap();
        assert!(service.health.bootstrap_blocked.load(Ordering::Acquire));
        service.health_tick().await.unwrap();
        assert!(!service.health.bootstrap_blocked.load(Ordering::Acquire));
        assert_eq!(service.dns_overview(None, 50).unwrap().cache.misses, 1);
        service.shutdown().await.unwrap();
    }
}
