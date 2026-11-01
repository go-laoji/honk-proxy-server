use serde::{Deserialize, Serialize};

/// Persistent task facts. Source references point to the existing operation's detailed result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: i64,
    pub version: u64,
    pub kind: String,
    pub source_ref: Option<String>,
    pub trigger: String,
    pub state: String,
    pub phase: String,
    pub terminal: bool,
    pub created_at_unix_ms: u64,
    pub started_at_unix_ms: Option<u64>,
    pub updated_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    pub configuration_revision: Option<i64>,
    pub configuration_fingerprint: Option<String>,
    pub resource_kind: Option<String>,
    pub resource_id: Option<String>,
    pub request_id: Option<String>,
    pub total: u64,
    pub completed: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub skipped: u64,
    pub removed: u64,
    pub error_code: Option<String>,
    pub result_ref: Option<String>,
    /// Only an explicitly sanitized, bounded diagnostic DTO; never operation input.
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TaskQuery {
    pub source_ref: Option<String>,
    pub expected_epoch: Option<String>,
    pub before: Option<i64>,
    pub kind: Option<String>,
    pub state: Option<String>,
    pub resource_kind: Option<String>,
    pub resource_id: Option<String>,
    pub request_id: Option<String>,
    pub created_after_unix_ms: Option<u64>,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasksOverview {
    pub epoch: String,
    pub observed_at_unix_ms: u64,
    pub records: Vec<TaskInfo>,
    pub next_before: Option<i64>,
    pub terminal_retention_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSubmission {
    pub request_id: String,
    pub operation: TaskOperation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "input",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TaskOperation {
    ConfigurationValidate(crate::ConfigInput),
    ConfigurationPrepare(crate::ConfigInput),
}

/// Structured metadata only: there is deliberately no arbitrary message/body/URL field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEvent {
    pub sequence: i64,
    pub recorded_epoch: String,
    pub occurred_at_unix_ms: u64,
    pub level: String,
    pub module: String,
    pub code: String,
    pub task_id: Option<i64>,
    pub task_version: Option<u64>,
    pub request_id: Option<String>,
    pub resource_kind: Option<String>,
    pub resource_id: Option<String>,
    pub node_id: Option<String>,
    pub provider_id: Option<String>,
    /// The actual accepted management peer, if the listener supplies it. No authenticated actor.
    pub peer: Option<String>,
    pub action: Option<String>,
    pub outcome: Option<String>,
    pub configuration_revision: Option<i64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EventQuery {
    pub cursor: Option<String>,
    pub level: Option<String>,
    pub module: Option<String>,
    pub task_id: Option<i64>,
    pub request_id: Option<String>,
    pub node_id: Option<String>,
    pub provider_id: Option<String>,
    pub after_unix_ms: Option<u64>,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsOverview {
    pub epoch: String,
    pub observed_at_unix_ms: u64,
    pub records: Vec<ServiceEvent>,
    /// Scanned through this cursor, including events excluded by filters.
    pub cursor: String,
    pub oldest_available_sequence: i64,
    pub head_sequence: i64,
    pub has_more: bool,
    pub reset_reason: Option<String>,
    pub retention_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum EventFrame {
    Snapshot(EventsOverview),
    Events(EventsOverview),
    Checkpoint(EventsOverview),
    Reset(EventsOverview),
    Unavailable { code: String },
}

pub const TASK_KINDS: &[&str] = &[
    "provider",
    "health",
    "activation",
    "configuration_prepare",
    "configuration_validate",
    "dns_diagnostic",
    "provider_diagnostic",
    "rules_diagnostic",
    "group_diagnostic",
];
pub const TASK_STATES: &[&str] = &[
    "queued",
    "running",
    "completed",
    "failed",
    "outcome_unknown",
    "discarded",
    "interrupted",
    "pending_restart",
];
pub const EVENT_MODULES: &[&str] = &[
    "provider",
    "health",
    "configuration",
    "diagnostic",
    "management",
    "runtime",
    "linux",
    "dns",
    "routing",
];

pub fn event_cursor(epoch: &str, sequence: i64) -> String {
    format!("{epoch}:{sequence}")
}
pub fn valid_task_source_ref(value: &str) -> bool {
    value.split_once(':').is_some_and(|(kind, id)| {
        ["provider", "health", "activation"].contains(&kind)
            && id
                .parse::<i64>()
                .is_ok_and(|n| n > 0 && n.to_string() == id)
    })
}
pub fn parse_event_cursor(value: &str) -> Option<(&str, i64)> {
    let (epoch, sequence) = value.split_once(':')?;
    if !crate::valid_connection_epoch(epoch)
        || sequence.is_empty()
        || (sequence.starts_with('0') && sequence.len() != 1)
        || !sequence.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((epoch, sequence.parse().ok()?))
}
