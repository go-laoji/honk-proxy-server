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
