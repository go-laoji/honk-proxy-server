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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_port: Option<u16>,
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
    pub health_evaluated_at_unix: Option<u64>,
    pub notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesSimulationInput {
    pub revision: Option<i64>,
    pub context: RoutingContext,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleCapabilities {
    pub local_process_available: bool,
    pub local_process_reason: String,
    pub udp_data_plane_available: bool,
    pub kernel_rule_revision: Option<i64>,
    pub kernel_rules_ready: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesOverview<C> {
    pub running_revision: i64,
    pub desired_revision: i64,
    pub viewed_revision: i64,
    pub viewed_parent_revision: Option<i64>,
    pub configuration: C,
    pub active_configuration: C,
    pub candidates: Vec<crate::ConfigRevision>,
    pub changes: Vec<crate::ConfigChange>,
    pub other_configuration_changes: bool,
    pub capabilities: RuleCapabilities,
}
