use crate::{RouteDecision, RoutingContext};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionChoice {
    pub node_id: String,
    pub provider_id: Option<String>,
    pub upstream_host: String,
    pub upstream_port: u16,
    pub upstream_protocol: String,
    pub expires_at_unix: Option<u64>,
    pub remaining_secs_at_admission: Option<u64>,
    pub minimum_remaining_secs: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSelection {
    pub group: Option<String>,
    pub strategy: String,
    pub reason: String,
    pub selected: Option<String>,
    pub session_reused: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub epoch: String,
    pub created_at_unix_ms: u64,
    pub ended_at_unix_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub context: RoutingContext,
    pub observed_domains_truncated: bool,
    pub routing: RouteDecision,
    pub choice: Option<ConnectionChoice>,
    pub selection: Option<ConnectionSelection>,
    pub state: String,
    pub phase: String,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub http_status: Option<u16>,
    pub error_code: Option<String>,
    pub close_reason: Option<String>,
    pub disconnect_requested: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionStatistics {
    pub epoch: String,
    pub started_at_unix_ms: u64,
    pub elapsed_ms: u64,
    pub active: usize,
    pub active_tracked: usize,
    pub active_untracked: usize,
    pub active_record_limit: usize,
    pub recent_record_limit: usize,
    pub started: u64,
    pub opened: u64,
    pub closed: u64,
    pub failed: u64,
    pub rejected: u64,
    pub disconnected: u64,
    pub untracked_total: u64,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub errors: std::collections::BTreeMap<String, u64>,
}
