use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SelectionTransport {
    #[default]
    Tcp,
    Udp,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SelectionQuery {
    pub node: Option<String>,
    pub group: Option<String>,
    pub inbound: Option<String>,
    pub source_ip: Option<IpAddr>,
    pub destination_host: Option<String>,
    pub destination_port: Option<u16>,
    pub transport: SelectionTransport,
    pub after: Option<String>,
    pub limit: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupCandidate {
    pub node_id: String,
    pub provider_id: Option<String>,
    pub protocol: String,
    pub tags: Vec<String>,
    pub eligible: bool,
    pub reasons: Vec<String>,
    pub remaining_secs: Option<u64>,
    pub latency_ms: Option<u64>,
    pub weight: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionDecision {
    pub group: Option<String>,
    pub strategy: String,
    pub selected: Option<String>,
    pub reason: String,
    pub session_reused: bool,
    pub session_remaining_secs: Option<u64>,
    pub candidates: Vec<GroupCandidate>,
    pub total: usize,
    pub eligible: usize,
    pub next_after: Option<String>,
    pub sessions: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupsOverview<C> {
    pub running_revision: i64,
    pub configuration: C,
    pub recent_choices: std::collections::HashMap<String, String>,
    pub default_selection: SelectionDecision,
}
