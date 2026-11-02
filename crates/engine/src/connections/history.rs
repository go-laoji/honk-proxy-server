//! Closed-flow metadata only. Sixty-second buckets, bounded dimensions and retention.
use super::*;
use hps_domain::{Correlations, FlowCorrelation};
const GROUPS: usize = 128;
const BUCKETS: usize = 61;
const RETENTION: Duration = Duration::from_secs(3600);
type Key = (Option<std::net::IpAddr>, Option<String>, String, u16);
#[derive(Default, Clone)]
struct Counts {
    closed: u64,
    failed: u64,
    up: u64,
    down: u64,
}
impl Counts {
    fn add(&mut self, other: &Self) {
        self.closed = self.closed.saturating_add(other.closed);
        self.failed = self.failed.saturating_add(other.failed);
        self.up = self.up.saturating_add(other.up);
        self.down = self.down.saturating_add(other.down);
    }
}
struct Bucket {
    at: Instant,
    index: u64,
    excluded: u64,
    groups: BTreeMap<Key, Counts>,
}
pub(super) struct History {
    origin: Instant,
    buckets: VecDeque<Bucket>,
}
impl Default for History {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            buckets: VecDeque::new(),
        }
    }
}
impl History {
    fn prune(&mut self, now: Instant) {
        while self
            .buckets
            .front()
            .is_some_and(|b| now.saturating_duration_since(b.at) > RETENTION)
            || self.buckets.len() > BUCKETS
        {
            self.buckets.pop_front();
        }
    }
    pub(super) fn record(&mut self, info: Option<&ConnectionInfo>) {
        self.record_at(info, Instant::now());
    }
    fn record_at(&mut self, info: Option<&ConnectionInfo>, now: Instant) {
        self.prune(now);
        let index = now.saturating_duration_since(self.origin).as_secs() / 60;
        if self.buckets.back().is_none_or(|b| b.index != index) {
            self.buckets.push_back(Bucket {
                at: now,
                index,
                excluded: 0,
                groups: BTreeMap::new(),
            });
        }
        let bucket = self.buckets.back_mut().unwrap();
        if let Some(info) = info {
            let key = (
                info.context.source_ip,
                info.context.inbound.clone(),
                info.context.destination_host.clone(),
                info.context.destination_port,
            );
            if bucket.groups.contains_key(&key) || bucket.groups.len() < GROUPS {
                bucket.groups.entry(key).or_default().add(&Counts {
                    closed: 1,
                    failed: u64::from(info.state == "failed"),
                    up: info.bytes_up,
                    down: info.bytes_down,
                });
                return;
            }
        }
        bucket.excluded = bucket.excluded.saturating_add(1);
    }
    pub(super) fn view(&mut self, window: u32) -> Correlations {
        self.view_at(window, Instant::now())
    }
    fn view_at(&mut self, window: u32, now: Instant) -> Correlations {
        self.prune(now);
        let mut grouped = BTreeMap::<Key, Counts>::new();
        let mut excluded = 0u64;
        for bucket in &self.buckets {
            // The boundary bucket is explicitly a minute-resolution aggregate.
            if now.saturating_duration_since(bucket.at).as_secs() > u64::from(window) + 59 {
                continue;
            }
            excluded = excluded.saturating_add(bucket.excluded);
            for (key, value) in &bucket.groups {
                grouped.entry(key.clone()).or_default().add(value);
            }
        }
        let total_groups = grouped.len();
        let mut groups = grouped
            .into_iter()
            .map(
                |((source_ip, inbound, destination_host, destination_port), c)| FlowCorrelation {
                    source_ip,
                    inbound,
                    destination_host,
                    destination_port,
                    closed: c.closed,
                    failed: c.failed,
                    bytes_up: c.up,
                    bytes_down: c.down,
                },
            )
            .collect::<Vec<_>>();
        groups.sort_by(|a, b| {
            b.closed.cmp(&a.closed).then_with(|| {
                (
                    &a.source_ip,
                    &a.inbound,
                    &a.destination_host,
                    a.destination_port,
                )
                    .cmp(&(
                        &b.source_ip,
                        &b.inbound,
                        &b.destination_host,
                        b.destination_port,
                    ))
            })
        });
        groups.truncate(200);
        Correlations {
            source: "closed_business_flow_metadata_in_current_daemon_epoch".into(),
            resolution_secs: 60,
            retention_secs: 3600,
            partial_boundary_bucket: true,
            unrepresented_closed_flows: excluded,
            group_limit_per_bucket: GROUPS,
            total_groups,
            groups_truncated: total_groups > 200,
            groups,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_windows_bound_dimensions_and_prune_by_monotonic_time() {
        let registry = Arc::new(NodeRegistry::default());
        let guard = super::super::tests::flow(&registry);
        let mut info = guard.0.record.snapshot().unwrap();
        info.state = "closed".into();
        info.bytes_up = 42;
        info.bytes_down = 84;
        let mut history = History::default();
        let start = history.origin;
        for n in 0..GROUPS + 1 {
            info.context.destination_host = format!("node-{n}.test");
            history.record_at(Some(&info), start);
        }
        history.record_at(None, start);
        let view = history.view_at(3600, start + Duration::from_secs(59));
        assert_eq!(view.groups.len(), GROUPS);
        assert_eq!(view.unrepresented_closed_flows, 2);
        assert!(
            view.groups
                .iter()
                .all(|v| v.bytes_up == 42 && v.bytes_down == 84)
        );
        assert!(
            history
                .view_at(3600, start + Duration::from_secs(3601))
                .groups
                .is_empty()
        );
    }
}
