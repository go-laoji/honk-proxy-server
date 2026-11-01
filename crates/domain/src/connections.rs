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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionQuery {
    pub snapshot: Option<String>,
    pub expected_epoch: Option<String>,
    pub id: Option<String>,
    pub source: Option<String>,
    pub source_port: Option<u16>,
    pub destination: Option<String>,
    pub destination_port: Option<u16>,
    pub inbound: Option<String>,
    pub protocol: Option<String>,
    pub transport: Option<String>,
    pub origin: Option<String>,
    pub node: Option<String>,
    pub provider: Option<String>,
    pub group: Option<String>,
    pub rule: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub created_after_unix_ms: Option<u64>,
    pub sort: Option<String>,
    pub offset: u32,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionsOverview {
    pub running_revision: i64,
    pub epoch: String,
    pub snapshot: String,
    pub snapshot_created_at_unix_ms: u64,
    pub snapshot_expires_at_unix_ms: u64,
    pub statistics: ConnectionStatistics,
    pub filtered_total: usize,
    pub offset: u32,
    pub next_offset: Option<u32>,
    pub records: Vec<ConnectionInfo>,
    pub inbounds: Vec<String>,
    pub groups: Vec<String>,
    pub providers: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDisconnectInput {
    pub id: String,
    pub expected_epoch: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDisconnectPreviewInput {
    pub snapshot: String,
    pub expected_epoch: String,
    /// Empty means all active records in this frozen, filtered snapshot, at most 500.
    #[serde(default)]
    pub ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDisconnectTarget {
    pub id: String,
    pub source_ip: Option<std::net::IpAddr>,
    pub source_port: Option<u16>,
    pub inbound: Option<String>,
    pub destination_host: String,
    pub destination_port: u16,
    pub node_id: Option<String>,
    pub configuration_revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDisconnectPreview {
    pub token: String,
    pub epoch: String,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub targets: Vec<ConnectionDisconnectTarget>,
    pub active_untracked_in_instance: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDisconnectConfirm {
    pub token: String,
    pub expected_epoch: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDisconnectResult {
    pub epoch: String,
    pub token: Option<String>,
    pub requested: usize,
    pub already_ended: usize,
    pub matched: usize,
    pub requested_at_unix_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionAttention {
    pub kind: String,
    pub code: String,
    pub severity: String,
    pub resource_id: Option<String>,
    pub observed_at_unix_ms: u64,
    pub href: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSummary {
    pub running_revision: i64,
    pub evaluated_at_unix_ms: u64,
    pub statistics: ConnectionStatistics,
    pub proxy: crate::ProxyStatus,
    pub attention: Vec<ConnectionAttention>,
}
/// Structural validation only; matching the live epoch remains the service's responsibility.
pub fn valid_connection_epoch(epoch: &str) -> bool {
    epoch.len() == 36
        && epoch.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
pub fn valid_connection_id(id: &str) -> bool {
    id.split_once(':').is_some_and(|(epoch, n)| {
        valid_connection_epoch(epoch)
            && n.len() == 20
            && n.parse::<u64>()
                .is_ok_and(|number| number > 0 && format!("{number:020}") == n)
    })
}
