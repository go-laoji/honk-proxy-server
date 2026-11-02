use crate::{ConfigError, ProxyConfig, ProxyProtocol};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupStrategy {
    Fixed,
    Failover,
    Latency,
    #[default]
    RoundRobin,
    Weighted,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKey {
    SourceIp,
    SourceDestination,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupSession {
    pub key: SessionKey,
    pub ttl_secs: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct GroupConfig {
    pub id: String,
    pub strategy: GroupStrategy,
    pub include_all: bool,
    pub members: Vec<String>,
    pub providers: Vec<String>,
    pub tags: Vec<String>,
    pub protocols: Vec<ProxyProtocol>,
    pub pinned_node: Option<String>,
    pub fallback_nodes: Vec<String>,
    pub weights: BTreeMap<String, u16>,
    pub session: Option<GroupSession>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SelectionConfig {
    pub groups: Vec<GroupConfig>,
    pub default_group: Option<String>,
    pub inbound_groups: BTreeMap<String, String>,
    pub node_tags: BTreeMap<String, Vec<String>>,
    pub provider_tags: BTreeMap<String, Vec<String>>,
    pub max_sessions: usize,
}
impl Default for SelectionConfig {
    fn default() -> Self {
        Self {
            groups: vec![],
            default_group: None,
            inbound_groups: BTreeMap::new(),
            node_tags: BTreeMap::new(),
            provider_tags: BTreeMap::new(),
            max_sessions: 10000,
        }
    }
}
pub fn valid_node_id(id: &str) -> bool {
    crate::valid_id(id)
        || id.strip_prefix("lease:").is_some_and(|n| {
            n.parse::<i64>()
                .is_ok_and(|id| id > 0 && id.to_string() == n)
        })
}
impl SelectionConfig {
    pub fn validate(&self, proxy: &ProxyConfig) -> Result<(), ConfigError> {
        let invalid =
            || ConfigError::Invalid("invalid group policy, reference or resource limit".into());
        if self.groups.len() > 256
            || self
                .groups
                .iter()
                .map(|g| g.members.len() + g.fallback_nodes.len() + g.weights.len())
                .sum::<usize>()
                > 100000
            || self.node_tags.len() > 100000
            || self.provider_tags.len() > 64
            || !(1..=100000).contains(&self.max_sessions)
            || self.inbound_groups.len() > 64
        {
            return Err(invalid());
        }
        let static_nodes = proxy
            .upstreams
            .iter()
            .map(|u| u.id.as_str())
            .collect::<HashSet<_>>();
        let known_node =
            |id: &str| valid_node_id(id) && (id.starts_with("lease:") || static_nodes.contains(id));
        let valid_tags =
            |tags: &[String]| tags.len() <= 32 && tags.iter().all(|t| crate::valid_id(t));
        let mut ids = HashSet::new();
        for group in &self.groups {
            if !crate::valid_id(&group.id)
                || !ids.insert(&group.id)
                || group.members.len() > 10000
                || group.providers.len() > 64
                || group.protocols.len() > 3
                || group.fallback_nodes.len() > 1000
                || group.weights.len() > 10000
                || !valid_tags(&group.tags)
                || !group.members.iter().all(|id| known_node(id))
                || !group.providers.iter().all(|id| crate::valid_id(id))
                || !group.fallback_nodes.iter().all(|id| known_node(id))
                || !group
                    .weights
                    .iter()
                    .all(|(id, w)| known_node(id) && (1..=1000).contains(w))
                || group.pinned_node.as_ref().is_some_and(|id| !known_node(id))
                || (group.strategy == GroupStrategy::Fixed && group.pinned_node.is_none())
                || (group.pinned_node.is_none() && !group.fallback_nodes.is_empty())
                || group
                    .session
                    .as_ref()
                    .is_some_and(|s| !(1..=86400).contains(&s.ttl_secs))
                || group.members.iter().collect::<HashSet<_>>().len() != group.members.len()
                || group.fallback_nodes.iter().collect::<HashSet<_>>().len()
                    != group.fallback_nodes.len()
            {
                return Err(invalid());
            }
        }
        if self
            .default_group
            .as_ref()
            .is_some_and(|id| !ids.contains(id))
            || self.inbound_groups.iter().any(|(inbound, group)| {
                !ids.contains(group) || !proxy.inbounds.iter().any(|i| &i.id == inbound)
            })
            || self
                .node_tags
                .iter()
                .any(|(id, tags)| !known_node(id) || !valid_tags(tags))
            || self
                .provider_tags
                .iter()
                .any(|(id, tags)| !crate::valid_id(id) || !valid_tags(tags))
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn validate_providers(
        &self,
        providers: &[crate::ProviderConfig],
    ) -> Result<(), ConfigError> {
        if self
            .groups
            .iter()
            .flat_map(|g| &g.providers)
            .chain(self.provider_tags.keys())
            .any(|id| !providers.iter().any(|p| &p.id == id))
        {
            return Err(ConfigError::Invalid(
                "group references an unknown provider".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionSave {
    pub configuration: SelectionConfig,
    pub expected_revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupUpsert {
    pub group: GroupConfig,
    pub expected_revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupDelete {
    pub group_id: String,
    pub expected_revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupPin {
    pub group_id: String,
    pub node_id: Option<String>,
    #[serde(default)]
    pub fallback_nodes: Vec<String>,
    pub expected_revision: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_rejects_dangling_references_implicit_fallback_and_unbounded_resources() {
        let text = "[[proxy.selection.groups]]\nid='g'\ninclude_all=true\n[proxy.selection]\ndefault_group='g'\n";
        let mut config = crate::DaemonConfig::from_toml(text).unwrap();
        config.proxy.selection.default_group = Some("missing".into());
        assert!(config.validate().is_err());
        config.proxy.selection.default_group = Some("g".into());
        config.proxy.selection.groups[0].providers = vec!["unknown".into()];
        assert!(config.validate().is_err());
        config.proxy.selection.groups[0].providers.clear();
        config.proxy.selection.groups[0].fallback_nodes = vec!["lease:1".into()];
        assert!(config.validate().is_err());
        config.proxy.selection.groups[0].pinned_node = Some("lease:42".into());
        assert!(config.validate().is_ok());
        config.proxy.selection.max_sessions = 0;
        assert!(config.validate().is_err());
        assert!(!valid_node_id("lease:0"));
        assert!(!valid_node_id("lease:-1"));
        assert!(valid_node_id("lease:42"));
    }
}
