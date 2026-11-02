//! Standard, long-lived subscription snapshots. Never a short-lived extraction lease.
use crate::{ConfigError, ProxyConfig, UpstreamConfig};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionFormat {
    #[default]
    Uri,
    Clash,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubscriptionConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub url: String,
    pub format: SubscriptionFormat,
    pub timeout_ms: u64,
    pub max_nodes: usize,
    pub snapshot: Option<SubscriptionSnapshot>,
}
impl Default for SubscriptionConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: true,
            url: String::new(),
            format: SubscriptionFormat::Uri,
            timeout_ms: 10000,
            max_nodes: 1000,
            snapshot: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionSnapshot {
    pub fetched_at_unix: i64,
    pub origin: String,
    pub nodes: Vec<UpstreamConfig>,
}
impl SubscriptionConfig {
    pub fn source_id(&self) -> String {
        format!("subscription:{}", self.id)
    }
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = || ConfigError::Invalid("invalid subscription definition or snapshot".into());
        let url = url::Url::parse(&self.url).map_err(|_| invalid())?;
        if !crate::valid_id(&self.id)
            || self.id.len() > 24
            || self.name.len() > 128
            || self.url.len() > 4096
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !(100..=120000).contains(&self.timeout_ms)
            || !(1..=10000).contains(&self.max_nodes)
        {
            return Err(invalid());
        }
        if let Some(snapshot) = &self.snapshot {
            let prefix = format!("sub:{}:", self.id);
            if snapshot.nodes.is_empty()
                || snapshot.nodes.len() > self.max_nodes
                || snapshot.fetched_at_unix <= 0
                || !["remote_get", "document_import"].contains(&snapshot.origin.as_str())
                || snapshot.nodes.iter().any(|n| {
                    !valid_subscription_node_id(&n.id)
                        || !n.id.starts_with(&prefix)
                        || n.expires_at_unix.is_some()
                        || n.ca_certificate.is_some()
                })
            {
                return Err(invalid());
            }
            // ProxyConfig only permits static ASCII IDs. Validate endpoint fields with temporary IDs.
            let mut nodes = snapshot.nodes.clone();
            let mut ids = std::collections::HashSet::new();
            for (i, n) in nodes.iter_mut().enumerate() {
                if !ids.insert(n.id.clone()) {
                    return Err(invalid());
                }
                n.id = format!("subscription-{i}");
            }
            ProxyConfig {
                upstreams: nodes,
                ..Default::default()
            }
            .validate()
            .map_err(|_| invalid())?;
        }
        Ok(())
    }
    pub fn redacted(&self) -> Self {
        let mut out = self.clone();
        out.url = "[REDACTED]".into();
        if let Some(snapshot) = &mut out.snapshot {
            for node in &mut snapshot.nodes {
                if let Some(c) = &mut node.credentials {
                    c.username = "[REDACTED]".into();
                    c.password = "[REDACTED]".into();
                }
            }
        }
        out
    }
}
pub fn valid_subscription_node_id(id: &str) -> bool {
    let Some(raw) = id.strip_prefix("sub:") else {
        return false;
    };
    let Some((source, hash)) = raw.split_once(':') else {
        return false;
    };
    crate::valid_id(source)
        && source.len() <= 24
        && hash.len() == 24
        && hash
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionSave {
    pub configuration: SubscriptionConfig,
    pub expected_revision: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionRefresh {
    pub expected_revision: i64,
    pub document: Option<String>,
}
impl std::fmt::Debug for SubscriptionRefresh {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionRefresh")
            .field("expected_revision", &self.expected_revision)
            .field("document", &self.document.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionsOverview {
    pub running_revision: i64,
    pub subscriptions: Vec<SubscriptionConfig>,
    pub expiry_semantics: String,
}

impl std::fmt::Debug for SubscriptionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionConfig")
            .field("id", &self.id)
            .field("url", &"[REDACTED]")
            .field("format", &self.format)
            .field("snapshot", &self.snapshot)
            .finish()
    }
}
