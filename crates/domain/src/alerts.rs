use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AlertSettings {
    pub enabled: bool,
    pub no_exit: bool,
    pub provider_failure: bool,
    pub capture_failure: bool,
    pub failure_rate: bool,
    pub cooldown_secs: u32,
    pub failure_min_count: u32,
    pub failure_percent: u32,
}
impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            no_exit: true,
            provider_failure: true,
            capture_failure: true,
            failure_rate: true,
            cooldown_secs: 60,
            failure_min_count: 20,
            failure_percent: 50,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertSave {
    pub expected_revision: i64,
    pub settings: AlertSettings,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertNotice {
    pub sequence: u64,
    pub occurred_at: i64,
    pub key: String,
    pub state: String,
    pub code: String,
    pub message: String,
    pub suppressed_changes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertFeed {
    pub epoch: String,
    pub oldest_sequence: Option<u64>,
    pub latest_sequence: u64,
    pub truncated: bool,
    pub notices: Vec<AlertNotice>,
    pub settings: AlertSettings,
    pub running_revision: i64,
    pub retention_limit: usize,
    pub resets_on_restart: bool,
    pub source: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AlertQuery {
    pub after: Option<u64>,
    pub limit: Option<u32>,
}
