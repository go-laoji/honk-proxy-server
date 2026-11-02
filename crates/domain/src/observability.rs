use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HealthHistoryQuery {
    pub node: Option<String>,
    pub since_unix: Option<i64>,
    pub before: Option<i64>,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthHistory {
    pub source: String,
    pub observed_at_unix: i64,
    pub since_unix: i64,
    pub oldest_retained_unix: Option<i64>,
    pub retained_task_limit: u32,
    pub retained_run_target: u32,
    pub records: Vec<crate::HealthRun>,
    pub next_before: Option<i64>,
    pub day_started_at: i64,
    pub used_probes: u32,
    pub daily_budget: u32,
    pub exit_ip_source: String,
    pub region_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleHitCounter {
    pub configuration_revision: i64,
    pub rule_id: Option<String>,
    pub hits: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleHitStatistics {
    pub epoch: String,
    pub since_unix_ms: u64,
    pub observed_at_unix_ms: u64,
    pub counter_limit: usize,
    pub untracked_hits: u64,
    pub counters: Vec<RuleHitCounter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsCounters {
    pub hits: u64,
    pub misses: u64,
    pub merged: u64,
    pub failures: u64,
    pub cache_entries: usize,
    pub cache_bytes: usize,
    pub in_flight: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendSample {
    pub at_unix_ms: u64,
    pub interval_ms: u64,
    pub active: usize,
    pub bytes_up_delta: u64,
    pub bytes_down_delta: u64,
    pub closed_delta: u64,
    pub failed_delta: u64,
    pub rejected_delta: u64,
    pub dns: DnsCounters,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowCorrelation {
    pub source_ip: Option<std::net::IpAddr>,
    pub inbound: Option<String>,
    pub destination_host: String,
    pub destination_port: u16,
    pub closed: u64,
    pub failed: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Correlations {
    pub source: String,
    pub resolution_secs: u32,
    pub retention_secs: u32,
    pub partial_boundary_bucket: bool,
    pub unrepresented_closed_flows: u64,
    pub group_limit_per_bucket: usize,
    pub total_groups: usize,
    pub groups_truncated: bool,
    pub groups: Vec<FlowCorrelation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ObservationQuery {
    pub window_secs: u32,
}
impl Default for ObservationQuery {
    fn default() -> Self {
        Self { window_secs: 3600 }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationOverview {
    pub epoch: String,
    pub source: String,
    pub observed_at_unix_ms: u64,
    pub requested_window_secs: u32,
    pub sample_interval_secs: u32,
    pub sample_limit: usize,
    pub oldest_sample_unix_ms: Option<u64>,
    pub resets_on_daemon_restart: bool,
    pub wall_clock_changed: bool,
    pub samples: Vec<TrendSample>,
    pub statistics: crate::ConnectionStatistics,
    pub rule_hits: RuleHitStatistics,
    pub correlations: Correlations,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleImportPreview {
    pub format_version: u32,
    pub checksum: String,
    pub rule_count: usize,
    pub validation: crate::ConfigValidation,
}
