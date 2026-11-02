use crate::ConfigError;
use hps_domain::{HealthTransport, IpFamily};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProbeProfile {
    pub transport: HealthTransport,
    pub family: IpFamily,
    pub enabled: bool,
    pub url: String,
    pub tls_server_name: Option<String>,
    pub ca_certificate: Option<PathBuf>,
    pub expected_status: Option<u16>,
    /// Read a bounded plain IP response from this explicitly configured target.
    #[serde(default, skip_serializing_if = "is_false")]
    pub capture_exit_ip: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}
impl Default for ProbeProfile {
    fn default() -> Self {
        Self {
            transport: HealthTransport::Tcp,
            family: IpFamily::Ipv4,
            enabled: true,
            url: "https://www.baidu.com/".into(),
            tls_server_name: None,
            ca_certificate: None,
            expected_status: None,
            capture_exit_ip: false,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HealthConfig {
    pub enabled: bool,
    pub profiles: Vec<ProbeProfile>,
    pub max_parallel: usize,
    pub queue_capacity: usize,
    pub timeout_ms: u64,
    pub interval_secs: u64,
    pub max_staleness_secs: u64,
    pub min_probe_margin_secs: u64,
    pub daily_probe_budget: u32,
    pub retry_initial_secs: u64,
    pub retry_max_secs: u64,
    pub traffic_failure_threshold: u32,
    pub max_header_bytes: usize,
}
impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            profiles: vec![
                ProbeProfile::default(),
                ProbeProfile {
                    family: IpFamily::Ipv6,
                    enabled: false,
                    ..ProbeProfile::default()
                },
            ],
            max_parallel: 8,
            queue_capacity: 256,
            timeout_ms: 5000,
            interval_secs: 60,
            max_staleness_secs: 180,
            min_probe_margin_secs: 5,
            daily_probe_budget: 5000,
            retry_initial_secs: 5,
            retry_max_secs: 300,
            traffic_failure_threshold: 2,
            max_header_bytes: 16384,
        }
    }
}
impl HealthConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = || {
            ConfigError::Invalid(
                "invalid health profile, timeout, concurrency or probe budget".into(),
            )
        };
        if self.profiles.len() > 4
            || !(1..=64).contains(&self.max_parallel)
            || !(16..=10000).contains(&self.queue_capacity)
            || !(100..=60000).contains(&self.timeout_ms)
            || !(1..=86400).contains(&self.interval_secs)
            || self.max_staleness_secs < self.interval_secs
            || self.max_staleness_secs > 86400
            || self.min_probe_margin_secs > 86400
            || !(1..=1000000).contains(&self.daily_probe_budget)
            || !(1..=86400).contains(&self.retry_initial_secs)
            || self.retry_max_secs < self.retry_initial_secs
            || self.retry_max_secs > 86400
            || !(1..=100).contains(&self.traffic_failure_threshold)
            || !(8192..=65536).contains(&self.max_header_bytes)
        {
            return Err(invalid());
        }
        let mut capabilities = HashSet::new();
        for profile in &self.profiles {
            if !capabilities.insert((profile.transport, profile.family)) {
                return Err(invalid());
            }
            if profile.transport == HealthTransport::Udp {
                if profile.enabled {
                    return Err(ConfigError::Invalid(
                        "UDP probing is not supported by this build".into(),
                    ));
                }
                continue;
            }
            let url = url::Url::parse(&profile.url).map_err(|_| invalid())?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || url.port_or_known_default().is_none_or(|p| p == 0)
                || profile
                    .expected_status
                    .is_some_and(|status| !(100..=599).contains(&status))
                || (url.scheme() == "http"
                    && (profile.tls_server_name.is_some() || profile.ca_certificate.is_some()))
                || profile
                    .tls_server_name
                    .as_ref()
                    .is_some_and(|name| !crate::valid_host(name))
            {
                return Err(invalid());
            }
            match (url.host(), profile.family) {
                (Some(url::Host::Ipv4(_)), IpFamily::Ipv6)
                | (Some(url::Host::Ipv6(_)), IpFamily::Ipv4) => return Err(invalid()),
                _ => {}
            }
        }
        if self.enabled
            && !self
                .profiles
                .iter()
                .any(|p| p.enabled && p.transport == HealthTransport::Tcp)
        {
            return Err(ConfigError::Invalid(
                "enabled health checking requires a TCP profile".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthSave {
    pub configuration: HealthConfig,
    pub expected_revision: i64,
}
