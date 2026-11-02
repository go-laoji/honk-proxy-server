use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticInput {
    pub expected_epoch: String,
    pub request_id: String,
    pub window_secs: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskControlInput {
    pub expected_epoch: String,
    pub expected_version: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRetryInput {
    pub expected_epoch: String,
    pub expected_version: u64,
    pub request_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskControls {
    pub task_id: i64,
    pub epoch: String,
    pub version: u64,
    pub can_cancel: bool,
    pub can_retry: bool,
    pub collection_finished: bool,
    pub result_persistence_failed: bool,
    pub reason: String,
    pub retry_input_retention_secs: u32,
    pub retry_of: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticBundle {
    pub format_version: u32,
    pub version: crate::VersionInfo,
    pub platform: String,
    pub configuration_revision: i64,
    pub epoch: String,
    pub collection_started_unix_ms: u64,
    pub collection_finished_unix_ms: u64,
    pub consistent_database_backup: bool,
    pub redaction: String,
    pub window_secs: u32,
    pub counter_scope: String,
    pub counter_started_unix_ms: u64,
    pub sample_count: usize,
    pub sample_interval_secs: u32,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub active_connections: usize,
    pub connection_limit: usize,
    pub inbounds_running: usize,
    pub health_probes_used: u32,
    pub health_daily_budget: u32,
    pub recent_probe_verdicts: std::collections::BTreeMap<String, u64>,
    pub recent_probes_examined: usize,
    pub recent_probe_limit: usize,
    pub error_counts: std::collections::BTreeMap<String, u64>,
    pub dns: crate::DnsCounters,
    pub linux_capture_configured: bool,
    pub linux_capture_ready: bool,
    pub unrepresented_closed_flows: u64,
    pub rule_counter_overflow_hits: u64,
    pub background_result_persistence_failures: u64,
}
