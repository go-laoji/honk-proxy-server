use super::*;
use hps_domain::{
    ConnectionStatistics, DnsCounters, ObservationOverview, ObservationQuery, TrendSample,
};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
const INTERVAL: u64 = 5;
const RETENTION: Duration = Duration::from_secs(3600);
const SAMPLES: usize = 721;
#[derive(Default)]
struct History {
    samples: VecDeque<(Instant, TrendSample)>,
    previous: Option<(Instant, ConnectionStatistics)>,
    wall_clock_changed: bool,
}
impl History {
    fn sample(
        &mut self,
        now: Instant,
        wall: u64,
        statistics: ConnectionStatistics,
        dns: DnsCounters,
    ) {
        if self
            .previous
            .as_ref()
            .is_some_and(|(_, s)| s.epoch != statistics.epoch)
        {
            *self = Self::default();
        }
        let (interval_ms, up, down, closed, failed, rejected) =
            self.previous
                .as_ref()
                .map_or((0, 0, 0, 0, 0, 0), |(at, old)| {
                    (
                        now.saturating_duration_since(*at)
                            .as_millis()
                            .min(u64::MAX as u128) as u64,
                        statistics.bytes_up.saturating_sub(old.bytes_up),
                        statistics.bytes_down.saturating_sub(old.bytes_down),
                        statistics.closed.saturating_sub(old.closed),
                        statistics.failed.saturating_sub(old.failed),
                        statistics.rejected.saturating_sub(old.rejected),
                    )
                });
        self.wall_clock_changed |= self.samples.back().is_some_and(|(_, s)| {
            wall < s.at_unix_ms || wall.abs_diff(s.at_unix_ms).abs_diff(interval_ms) > 2000
        });
        self.samples.push_back((
            now,
            TrendSample {
                at_unix_ms: wall,
                interval_ms,
                active: statistics.active,
                bytes_up_delta: up,
                bytes_down_delta: down,
                closed_delta: closed,
                failed_delta: failed,
                rejected_delta: rejected,
                dns,
            },
        ));
        self.previous = Some((now, statistics));
        self.prune(now);
    }
    fn prune(&mut self, now: Instant) {
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > RETENTION)
            || self.samples.len() > SAMPLES
        {
            self.samples.pop_front();
        }
    }
}
pub(super) struct ObservationRuntime {
    started: AtomicBool,
    history: Mutex<History>,
    slots: Arc<tokio::sync::Semaphore>,
}
impl Default for ObservationRuntime {
    fn default() -> Self {
        Self {
            started: AtomicBool::new(false),
            history: Mutex::new(History::default()),
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }
}
impl Supervisor {
    pub(super) fn start_observer(self: &Arc<Self>) {
        if self.observability.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(self);
        let cancel = self.provider_cancel.clone();
        self.provider_tasks.spawn(async move {
            let mut timer=tokio::time::interval(Duration::from_secs(INTERVAL));timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {tokio::select! {biased;_=cancel.cancelled()=>break,_=timer.tick()=>{
                let Some(service)=weak.upgrade() else {break;};
                let current=service.current();
                service.observability.history.lock().unwrap().sample(Instant::now(),hps_storage::tasks::unix_ms(),current.engine.node_registry().connection_statistics(),current.dns.statistics());
            }}}
        });
    }
    pub async fn observation_overview(
        self: &Arc<Self>,
        query: ObservationQuery,
    ) -> Result<ObservationOverview, ApplicationError> {
        if !(60..=3600).contains(&query.window_secs) {
            return Err(ApplicationError::ObservationInput);
        }
        let permit = self
            .observability
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::ObservationCapacity)?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let now = Instant::now();
            let current = service.current();
            let registry = current.engine.node_registry();
            let mut history = service.observability.history.lock().unwrap();
            history.prune(now);
            let samples = history
                .samples
                .iter()
                .filter(|(at, _)| {
                    now.saturating_duration_since(*at).as_secs() <= u64::from(query.window_secs)
                })
                .map(|(_, s)| s.clone())
                .collect::<Vec<_>>();
            let oldest_sample_unix_ms = samples.first().map(|s| s.at_unix_ms);
            let wall_clock_changed = history.wall_clock_changed;
            drop(history);
            ObservationOverview {
                epoch: service.event_epoch().into(),
                source: "monotonic_samples_of_business_counters_in_current_daemon_epoch".into(),
                observed_at_unix_ms: hps_storage::tasks::unix_ms(),
                requested_window_secs: query.window_secs,
                sample_interval_secs: INTERVAL as u32,
                sample_limit: SAMPLES,
                oldest_sample_unix_ms,
                resets_on_daemon_restart: true,
                wall_clock_changed,
                samples,
                statistics: registry.connection_statistics(),
                rule_hits: registry.rule_hit_statistics(),
                correlations: registry.connection_correlations(query.window_secs),
            }
        })
        .await
        .map_err(|_| ApplicationError::Worker)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_counter_deltas_bound_samples_and_use_monotonic_retention_across_wall_clock_rollback() {
        let engine = Engine::new(hps_config::ProxyConfig::default()).unwrap();
        let stats = engine.node_registry().connection_statistics();
        let resolver = hps_dns::Resolver::prepare(
            hps_config::DnsConfig::default(),
            &hps_config::ProxyConfig::default(),
            None,
        )
        .unwrap();
        let dns = resolver.statistics();
        let mut history = History::default();
        let start = Instant::now();
        history.sample(start, 10_000, stats.clone(), dns.clone());
        let mut next = stats.clone();
        next.bytes_down = 200;
        next.bytes_up = 100;
        next.closed = 2;
        history.sample(
            start + Duration::from_secs(5),
            9_000,
            next.clone(),
            dns.clone(),
        );
        let last = &history.samples.back().unwrap().1;
        assert_eq!(last.interval_ms, 5000);
        assert_eq!(last.bytes_down_delta, 200);
        assert_eq!(last.bytes_up_delta, 100);
        assert!(history.wall_clock_changed);
        for n in 2..1000 {
            history.sample(
                start + Duration::from_secs(n * 5),
                n * 5000,
                next.clone(),
                dns.clone(),
            );
        }
        assert!(history.samples.len() <= SAMPLES);
        assert!(history.samples.front().unwrap().0 >= start + Duration::from_secs(1395));
        next.epoch = "new-epoch".into();
        history.sample(start + Duration::from_secs(5005), 5_005_000, next, dns);
        assert_eq!(history.samples.len(), 1);
        assert_eq!(history.samples[0].1.interval_ms, 0);
    }
}
