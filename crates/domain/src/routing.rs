use crate::{SelectionDecision, SelectionTransport};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteAction {
    #[default]
    Default,
    Node {
        id: String,
    },
    Group {
        id: String,
    },
    Direct,
    Reject,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FlowOrigin {
    #[default]
    Explicit,
    LinuxLocal,
    Gateway,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub command: String,
    pub cgroup_id: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingContext {
    pub destination_host: String,
    pub destination_port: u16,
    #[serde(default)]
    pub transport: SelectionTransport,
    pub source_ip: Option<IpAddr>,
    pub inbound: Option<String>,
    pub inbound_protocol: Option<String>,
    #[serde(default)]
    pub observed_domains: Vec<String>,
    #[serde(default)]
    pub origin: FlowOrigin,
    pub process: Option<ProcessIdentity>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleEvaluation {
    pub id: String,
    pub matched: bool,
    pub reasons: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub configuration_revision: i64,
    pub evaluated_at_unix: u64,
    pub rule_id: Option<String>,
    pub action: RouteAction,
    pub domain: Option<String>,
    pub domain_source: String,
    pub trace: Vec<RuleEvaluation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSimulation {
    pub decision: RouteDecision,
    pub selection: Option<SelectionDecision>,
    pub action_supported: bool,
    pub health_evaluated_at_unix: u64,
    pub notes: Vec<String>,
}
