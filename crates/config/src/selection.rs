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
    Score,
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
    pub groups: Vec<String>,
    pub score: ScoreConfig,
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
pub struct ScoreConfig {
    pub latency_weight: u16,
    pub lifetime_weight: u16,
    pub priority_weight: u16,
    pub latency_ceiling_ms: u64,
    pub lifetime_target_secs: u64,
    pub missing_evidence_penalty: u16,
}
impl Default for ScoreConfig {
    fn default() -> Self {
        Self {
            latency_weight: 60,
            lifetime_weight: 30,
            priority_weight: 10,
            latency_ceiling_ms: 2000,
            lifetime_target_secs: 300,
            missing_evidence_penalty: 250,
        }
    }
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
        || crate::valid_subscription_node_id(id)
        || id.strip_prefix("lease:").is_some_and(|n| {
            n.parse::<i64>()
                .is_ok_and(|id| id > 0 && id.to_string() == n)
        })
}
pub fn valid_source_id(id: &str) -> bool {
    crate::valid_id(id)
        || id
            .strip_prefix("subscription:")
            .is_some_and(|id| crate::valid_id(id) && id.len() <= 24)
}
impl SelectionConfig {
    pub fn validate(&self, proxy: &ProxyConfig) -> Result<(), ConfigError> {
        let invalid =
            || ConfigError::Invalid("invalid group policy, reference or resource limit".into());
        if self.groups.len() > 256
            || self
                .groups
                .iter()
                .map(|g| {
                    g.members.len() + g.groups.len() + g.fallback_nodes.len() + g.weights.len()
                })
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
        let known_node = |id: &str| {
            valid_node_id(id)
                && (id.starts_with("lease:")
                    || crate::valid_subscription_node_id(id)
                    || static_nodes.contains(id))
        };
        let valid_tags =
            |tags: &[String]| tags.len() <= 32 && tags.iter().all(|t| crate::valid_id(t));
        let mut ids = HashSet::new();
        for group in &self.groups {
            if !crate::valid_id(&group.id)
                || !ids.insert(&group.id)
                || group.members.len() > 10000
                || group.groups.len() > 64
                || group.groups.iter().collect::<HashSet<_>>().len() != group.groups.len()
                || u32::from(group.score.latency_weight)
                    + u32::from(group.score.lifetime_weight)
                    + u32::from(group.score.priority_weight)
                    == 0
                || [
                    group.score.latency_weight,
                    group.score.lifetime_weight,
                    group.score.priority_weight,
                    group.score.missing_evidence_penalty,
                ]
                .iter()
                .any(|w| *w > 1000)
                || !(1..=60000).contains(&group.score.latency_ceiling_ms)
                || !(1..=86400).contains(&group.score.lifetime_target_secs)
                || group.providers.len() > 64
                || group.protocols.len() > 3
                || group.fallback_nodes.len() > 1000
                || group.weights.len() > 10000
                || !valid_tags(&group.tags)
                || !group.members.iter().all(|id| known_node(id))
                || !group.providers.iter().all(|id| valid_source_id(id))
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
        // Bound both recursion depth and diamond-shaped expansion work. A cycle fails before publication.
        fn visit(
            config: &SelectionConfig,
            id: &str,
            path: &mut Vec<String>,
            budget: &mut usize,
        ) -> Result<(), ConfigError> {
            if path.len() >= 8 || path.iter().any(|p| p == id) || *budget == 0 {
                return Err(ConfigError::Invalid(
                    "nested group cycle, depth or expansion budget exceeded".into(),
                ));
            }
            *budget -= 1;
            let group = config
                .groups
                .iter()
                .find(|g| g.id == id)
                .ok_or_else(|| ConfigError::Invalid("unknown nested group".into()))?;
            path.push(id.to_owned());
            for child in &group.groups {
                visit(config, child, path, budget)?;
            }
            path.pop();
            Ok(())
        }
        let mut budget = 4096;
        for group in &self.groups {
            visit(self, &group.id, &mut vec![], &mut budget)?;
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
                .any(|(id, tags)| !valid_source_id(id) || !valid_tags(tags))
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn validate_sources(
        &self,
        providers: &[crate::ProviderConfig],
        subscriptions: &[crate::SubscriptionConfig],
    ) -> Result<(), ConfigError> {
        if self
            .groups
            .iter()
            .flat_map(|g| &g.providers)
            .chain(self.provider_tags.keys())
            .any(|id| {
                !providers.iter().any(|p| &p.id == id)
                    && !subscriptions.iter().any(|p| p.source_id() == *id)
            })
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
    #[test]
    fn nested_policy_rejects_cycles_unknown_children_depth_and_expansion_budget() {
        let mut proxy = ProxyConfig::default();
        proxy.selection.groups = vec![
            GroupConfig {
                id: "a".into(),
                groups: vec!["b".into()],
                ..Default::default()
            },
            GroupConfig {
                id: "b".into(),
                include_all: true,
                ..Default::default()
            },
        ];
        assert!(proxy.validate().is_ok());
        proxy.selection.groups[1].groups = vec!["a".into()];
        assert!(proxy.validate().is_err());
        proxy.selection.groups[1].groups = vec!["missing".into()];
        assert!(proxy.validate().is_err());
        proxy.selection.groups = (0..9)
            .map(|i| GroupConfig {
                id: format!("g{i}"),
                groups: if i < 8 {
                    vec![format!("g{}", i + 1)]
                } else {
                    vec![]
                },
                ..Default::default()
            })
            .collect();
        assert!(proxy.validate().is_err());
        proxy.selection.groups = (0..8)
            .map(|i| GroupConfig {
                id: format!("g{i}"),
                groups: if i < 7 {
                    vec![format!("g{}", i + 1)]
                } else {
                    vec![]
                },
                ..Default::default()
            })
            .collect();
        assert!(proxy.validate().is_ok());
        // Dense acyclic expansion has legal depth but exceeds work budget.
        proxy.selection.groups = (0..35)
            .map(|i| GroupConfig {
                id: format!("g{i}"),
                groups: ((i / 5 + 1) * 5..((i / 5 + 2) * 5).min(35))
                    .map(|j| format!("g{j}"))
                    .collect(),
                ..Default::default()
            })
            .collect();
        assert!(proxy.validate().is_err());
        proxy.selection.groups = vec![GroupConfig {
            id: "score".into(),
            strategy: GroupStrategy::Score,
            ..Default::default()
        }];
        proxy.selection.groups[0].score.latency_weight = 0;
        proxy.selection.groups[0].score.lifetime_weight = 0;
        proxy.selection.groups[0].score.priority_weight = 0;
        assert!(proxy.validate().is_err());
    }
}
