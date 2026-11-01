use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonConfig {
    pub instance_name: String,
    pub management_listen: SocketAddr,
    pub log_filter: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            instance_name: "honk-proxy-server".into(),
            management_listen: SocketAddr::from(([127, 0, 0, 1], 17900)),
            log_filter: "info".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid TOML configuration: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("{0}")]
    Invalid(String),
}

impl DaemonConfig {
    pub fn from_toml(input: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
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
        Ok(())
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
