use crate::{ConfigError, ProxyConfig};
use hps_domain::{RouteAction, SelectionTransport};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProcessCondition {
    pub pid: Option<u32>,
    pub command: Option<String>,
    pub cgroup_id: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RuleConditions {
    pub domains: Vec<String>,
    pub domain_suffixes: Vec<String>,
    pub destination_cidrs: Vec<String>,
    pub destination_ports: Vec<PortRange>,
    pub transports: Vec<SelectionTransport>,
    pub inbound_protocols: Vec<String>,
    pub inbounds: Vec<String>,
    pub source_cidrs: Vec<String>,
    pub process: Option<ProcessCondition>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    pub id: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub conditions: RuleConditions,
    pub action: RouteAction,
}
const fn enabled() -> bool {
    true
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct RoutingConfig {
    pub rules: Vec<RuleConfig>,
    pub default_action: RouteAction,
    pub use_dns_observations: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulesSave {
    pub configuration: RoutingConfig,
    pub expected_revision: i64,
    pub base_revision: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleUpsert {
    pub rule: RuleConfig,
    pub expected_revision: i64,
    pub base_revision: Option<i64>,
    pub index: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDelete {
    pub id: String,
    pub expected_revision: i64,
    pub base_revision: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleToggle {
    pub id: String,
    pub enabled: bool,
    pub expected_revision: i64,
    pub base_revision: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleOrder {
    pub ids: Vec<String>,
    pub expected_revision: i64,
    pub base_revision: Option<i64>,
}
impl RoutingConfig {
    pub fn validate(&self, proxy: &ProxyConfig) -> Result<(), ConfigError> {
        let invalid = || {
            ConfigError::Invalid(
                "invalid routing condition, action, reference or resource limit".into(),
            )
        };
        if self.rules.len() > 1024 {
            return Err(invalid());
        }
        let nodes = proxy
            .upstreams
            .iter()
            .map(|u| u.id.as_str())
            .collect::<HashSet<_>>();
        let groups = proxy
            .selection
            .groups
            .iter()
            .map(|g| g.id.as_str())
            .collect::<HashSet<_>>();
        let inbounds = proxy
            .inbounds
            .iter()
            .map(|i| i.id.as_str())
            .collect::<HashSet<_>>();
        let action_valid = |a: &RouteAction| match a {
            RouteAction::Node { id } => {
                crate::valid_node_id(id)
                    && (id.starts_with("lease:") || nodes.contains(id.as_str()))
            }
            RouteAction::Group { id } => groups.contains(id.as_str()),
            _ => true,
        };
        if !action_valid(&self.default_action) {
            return Err(invalid());
        }
        let mut ids = HashSet::new();
        let mut total = 0usize;
        for rule in &self.rules {
            let c = &rule.conditions;
            let counts = [
                c.domains.len(),
                c.domain_suffixes.len(),
                c.destination_cidrs.len(),
                c.destination_ports.len(),
                c.source_cidrs.len(),
                c.inbounds.len(),
            ];
            total += counts.iter().sum::<usize>();
            if !crate::valid_id(&rule.id)
                || !ids.insert(&rule.id)
                || !action_valid(&rule.action)
                || counts.iter().any(|&n| n > 256)
                || total > 100000
                || c.transports.len() > 2
                || c.inbound_protocols.len() > 4
                || c.domains
                    .iter()
                    .chain(&c.domain_suffixes)
                    .any(|s| s.parse::<std::net::IpAddr>().is_ok() || !crate::valid_host(s))
                || c.destination_cidrs
                    .iter()
                    .chain(&c.source_cidrs)
                    .any(|s| s.len() > 64 || s.parse::<ipnet::IpNet>().is_err())
                || c.destination_ports
                    .iter()
                    .any(|p| p.start == 0 || p.end < p.start)
                || c.inbounds.iter().any(|id| !inbounds.contains(id.as_str()))
                || c.inbound_protocols
                    .iter()
                    .any(|p| !matches!(p.as_str(), "http" | "https" | "socks5" | "transparent"))
                || c.process.as_ref().is_some_and(|p| {
                    (p.pid.is_none() && p.command.is_none() && p.cgroup_id.is_none())
                        || p.pid == Some(0)
                        || p.cgroup_id == Some(0)
                        || p.command.as_ref().is_some_and(|s| {
                            s.is_empty() || s.len() > 15 || s.chars().any(char::is_control)
                        })
                })
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_references_cidrs_ports_and_account_fields_are_validated() {
        let mut config = ProxyConfig::default();
        config.routing.rules.push(RuleConfig {
            id: "r".into(),
            enabled: true,
            conditions: RuleConditions::default(),
            action: RouteAction::Direct,
        });
        assert!(config.validate().is_ok());
        config.routing.rules[0].action = RouteAction::Group {
            id: "missing".into(),
        };
        assert!(config.validate().is_err());
        config.routing.rules[0].action = RouteAction::Node {
            id: "missing".into(),
        };
        assert!(config.validate().is_err());
        config.routing.rules[0].action = RouteAction::Direct;
        config.routing.rules[0].conditions.destination_cidrs = vec!["192.0.2.0/33".into()];
        assert!(config.validate().is_err());
        config.routing.rules[0].conditions.destination_cidrs = vec!["2001:db8::/32".into()];
        config.routing.rules[0].conditions.destination_ports = vec![PortRange {
            start: 443,
            end: 80,
        }];
        assert!(config.validate().is_err());
        config.routing.rules[0].conditions.destination_ports = vec![PortRange {
            start: 443,
            end: 443,
        }];
        assert!(config.validate().is_ok());
        assert!(crate::DaemonConfig::from_toml("[[proxy.routing.rules]]\nid='r'\n[proxy.routing.rules.action]\nkind='direct'\n[proxy.routing.rules.conditions]\naccount='admin'\n").is_err());
    }
}
