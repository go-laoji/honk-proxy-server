use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::Path};

mod subscription;
pub use subscription::*;
mod health;
pub use health::*;
mod dns;
pub use dns::*;
mod provider;
pub use provider::*;
mod proxy;
pub use proxy::*;
mod selection;
pub use selection::*;
mod routing;
pub use routing::*;
mod linux;
pub use linux::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    pub instance_name: String,
    pub management_listen: SocketAddr,
    pub log_filter: String,
    pub management_tls: Option<ServerTls>,
    pub retention: RetentionConfig,
    pub alerts: hps_domain::AlertSettings,
    pub proxy: ProxyConfig,
    pub providers: Vec<ProviderConfig>,
    pub subscriptions: Vec<SubscriptionConfig>,
    pub health: HealthConfig,
    pub dns: DnsConfig,
    pub linux_capture: Option<LinuxCaptureConfig>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            instance_name: "honk-proxy-server".into(),
            management_listen: SocketAddr::from(([127, 0, 0, 1], 17900)),
            log_filter: "info".into(),
            management_tls: None,
            retention: RetentionConfig::default(),
            alerts: Default::default(),
            proxy: ProxyConfig::default(),
            providers: vec![],
            subscriptions: vec![],
            health: HealthConfig::default(),
            dns: DnsConfig::default(),
            linux_capture: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid TOML configuration; check the file syntax and field types")]
    Parse,
    #[error("{0}")]
    Invalid(String),
}

impl DaemonConfig {
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        // Parser diagnostics can contain the original line, including passwords.
        let config: Self = toml::from_str(input).map_err(|_| ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let mut config = Self::from_toml(&std::fs::read_to_string(path)?)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()?;
        config.resolve_paths(&parent);
        Ok(config)
    }

    pub fn resolve_paths(&mut self, parent: &Path) {
        if let Some(capture) = &mut self.linux_capture
            && capture.object.is_relative()
        {
            capture.object = parent.join(&capture.object);
        }
        if let Some(capture) = &mut self.dns.capture
            && capture.object.is_relative()
        {
            capture.object = parent.join(&capture.object);
        }
        for upstream in &mut self.dns.upstreams {
            if let Some(path) = &mut upstream.ca_certificate
                && path.is_relative()
            {
                *path = parent.join(&*path);
            }
        }
        for profile in &mut self.health.profiles {
            if let Some(path) = &mut profile.ca_certificate
                && path.is_relative()
            {
                *path = parent.join(&*path);
            }
        }
        if let Some(tls) = &mut self.management_tls {
            if tls.certificate.is_relative() {
                tls.certificate = parent.join(&tls.certificate);
            }
            if tls.private_key.is_relative() {
                tls.private_key = parent.join(&tls.private_key);
            }
        }
        for inbound in &mut self.proxy.inbounds {
            if let Some(tls) = &mut inbound.tls {
                if tls.certificate.is_relative() {
                    tls.certificate = parent.join(&tls.certificate);
                }
                if tls.private_key.is_relative() {
                    tls.private_key = parent.join(&tls.private_key);
                }
            }
        }
        for upstream in &mut self.proxy.upstreams {
            if let Some(ca) = &mut upstream.ca_certificate
                && ca.is_relative()
            {
                *ca = parent.join(&*ca);
            }
        }
    }

    pub fn restore_redacted(&mut self, original: &Self) -> Result<(), ConfigError> {
        for provider in &mut self.providers {
            provider
                .restore_redacted(original.providers.iter().find(|old| old.id == provider.id))?;
        }
        for subscription in &mut self.subscriptions {
            if subscription.url == "[REDACTED]" {
                subscription.url = original
                    .subscriptions
                    .iter()
                    .find(|old| old.id == subscription.id)
                    .ok_or_else(|| {
                        ConfigError::Invalid("redacted subscription URL has no reference".into())
                    })?
                    .url
                    .clone();
            }
            if let Some(snapshot) = &mut subscription.snapshot {
                for node in &mut snapshot.nodes {
                    if let Some(c) = &mut node.credentials {
                        let old = original
                            .subscriptions
                            .iter()
                            .find(|s| s.id == subscription.id)
                            .and_then(|s| s.snapshot.as_ref())
                            .and_then(|s| s.nodes.iter().find(|n| n.id == node.id))
                            .and_then(|n| n.credentials.as_ref());
                        if c.username == "[REDACTED]" {
                            c.username = old
                                .ok_or_else(|| {
                                    ConfigError::Invalid(
                                        "redacted subscription credentials have no reference"
                                            .into(),
                                    )
                                })?
                                .username
                                .clone();
                        }
                        if c.password == "[REDACTED]" {
                            c.password = old
                                .ok_or_else(|| {
                                    ConfigError::Invalid(
                                        "redacted subscription credentials have no reference"
                                            .into(),
                                    )
                                })?
                                .password
                                .clone();
                        }
                    }
                }
            }
        }
        for upstream in &mut self.proxy.upstreams {
            if let Some(credentials) = &mut upstream.credentials {
                let old = original
                    .proxy
                    .upstreams
                    .iter()
                    .find(|old| old.id == upstream.id)
                    .and_then(|old| old.credentials.as_ref());
                if credentials.username == "[REDACTED]" {
                    credentials.username = old
                        .ok_or_else(|| {
                            ConfigError::Invalid(
                                "redacted credentials have no existing reference".into(),
                            )
                        })?
                        .username
                        .clone();
                }
                if credentials.password == "[REDACTED]" {
                    credentials.password = old
                        .ok_or_else(|| {
                            ConfigError::Invalid(
                                "redacted credentials have no existing reference".into(),
                            )
                        })?
                        .password
                        .clone();
                }
            }
        }
        self.validate()
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(5..=3600).contains(&self.alerts.cooldown_secs)
            || !(1..=100000).contains(&self.alerts.failure_min_count)
            || !(1..=100).contains(&self.alerts.failure_percent)
        {
            return Err(ConfigError::Invalid(
                "alert thresholds are out of range".into(),
            ));
        }
        if let Some(capture) = &self.linux_capture {
            capture.validate()?;
        }
        if !(10..=10000).contains(&self.retention.config_versions)
            || !(1..=3650).contains(&self.retention.activity_days)
            || !(1..=3650).contains(&self.retention.task_days)
        {
            return Err(ConfigError::Invalid(
                "retention limits are out of range".into(),
            ));
        }
        if self.instance_name.trim().is_empty() || self.instance_name.len() > 128 {
            return Err(ConfigError::Invalid(
                "instance_name must contain 1–128 bytes".into(),
            ));
        }
        if self.log_filter.trim().is_empty() {
            return Err(ConfigError::Invalid("log_filter cannot be empty".into()));
        }
        if self.providers.len() > 64 {
            return Err(ConfigError::Invalid("provider count exceeds 64".into()));
        }
        let mut ids = std::collections::HashSet::new();
        for provider in &self.providers {
            provider.validate()?;
            if !ids.insert(&provider.id) {
                return Err(ConfigError::Invalid("duplicate provider ID".into()));
            }
        }
        if self.subscriptions.len() > 32
            || self
                .subscriptions
                .iter()
                .filter_map(|s| s.snapshot.as_ref())
                .map(|s| s.nodes.len())
                .sum::<usize>()
                > 20000
        {
            return Err(ConfigError::Invalid(
                "subscription resource budget exceeded".into(),
            ));
        }
        let mut subscription_ids = std::collections::HashSet::new();
        for subscription in &self.subscriptions {
            subscription.validate()?;
            if !subscription_ids.insert(&subscription.id) {
                return Err(ConfigError::Invalid("duplicate subscription ID".into()));
            }
        }
        self.proxy.validate()?;
        self.proxy
            .selection
            .validate_sources(&self.providers, &self.subscriptions)?;
        self.health.validate()?;
        self.dns.validate(&self.proxy)?;
        Ok(())
    }

    pub fn redacted(&self) -> Self {
        let mut config = self.clone();
        config.subscriptions = self
            .subscriptions
            .iter()
            .map(SubscriptionConfig::redacted)
            .collect();
        config.providers = self
            .providers
            .iter()
            .map(ProviderConfig::redacted)
            .collect();
        for upstream in &mut config.proxy.upstreams {
            if let Some(credentials) = &mut upstream.credentials {
                credentials.username = "[REDACTED]".into();
                credentials.password = "[REDACTED]".into();
            }
        }
        config
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionConfig {
    pub config_versions: u32,
    pub activity_days: u32,
    pub task_days: u32,
}
impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            config_versions: 200,
            activity_days: 7,
            task_days: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_rejects_obsolete_auth_and_unknown_options() {
        assert!(DaemonConfig::from_toml("password = 'unused'").is_err());
        assert!(DaemonConfig::from_toml("instance_name = ' '").is_err());
        assert!(DaemonConfig::from_toml("management_listen = 'invalid'").is_err());
        assert!(DaemonConfig::from_toml("").is_ok());
    }
}
