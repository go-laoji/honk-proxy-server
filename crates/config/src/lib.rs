use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::Path};

mod proxy;
pub use proxy::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    pub instance_name: String,
    pub management_listen: SocketAddr,
    pub log_filter: String,
    pub management_tls: Option<ServerTls>,
    pub proxy: ProxyConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            instance_name: "honk-proxy-server".into(),
            management_listen: SocketAddr::from(([127, 0, 0, 1], 17900)),
            log_filter: "info".into(),
            management_tls: None,
            proxy: ProxyConfig::default(),
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
        if self.instance_name.trim().is_empty() || self.instance_name.len() > 128 {
            return Err(ConfigError::Invalid(
                "instance_name must contain 1–128 bytes".into(),
            ));
        }
        if self.log_filter.trim().is_empty() {
            return Err(ConfigError::Invalid("log_filter cannot be empty".into()));
        }
        self.proxy.validate()?;
        Ok(())
    }

    pub fn redacted(&self) -> Self {
        let mut config = self.clone();
        for upstream in &mut config.proxy.upstreams {
            if let Some(credentials) = &mut upstream.credentials {
                credentials.username = "[REDACTED]".into();
                credentials.password = "[REDACTED]".into();
            }
        }
        config
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
