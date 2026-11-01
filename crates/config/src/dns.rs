use crate::{ConfigError, ProxyConfig};
use hps_net::ResolvePurpose;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsProtocol {
    System,
    Udp,
    Tcp,
    Tls,
    Https,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsUpstream {
    pub id: String,
    pub protocol: DnsProtocol,
    /// Socket addresses are always explicit. Hostnames belong in server_name;
    /// this removes the need to resolve a resolver using itself.
    #[serde(default)]
    pub addresses: Vec<SocketAddr>,
    pub server_name: Option<String>,
    pub https_path: Option<String>,
    pub ca_certificate: Option<PathBuf>,
    pub proxy_id: Option<String>,
    #[serde(default)]
    pub proxy_bootstrap_ips: Vec<IpAddr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsRoute {
    pub id: String,
    pub upstream: String,
    pub domain: Option<String>,
    #[serde(default)]
    pub suffix: bool,
    #[serde(default)]
    pub query_types: Vec<String>,
    #[serde(default)]
    pub purposes: Vec<ResolvePurpose>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DnsConfig {
    pub enabled: bool,
    pub upstreams: Vec<DnsUpstream>,
    pub routes: Vec<DnsRoute>,
    pub default_upstream: String,
    pub listen: Option<SocketAddr>,
    pub capture: Option<DnsCaptureConfig>,
    pub timeout_ms: u64,
    pub max_parallel: usize,
    pub cache_entries: usize,
    pub cache_bytes: usize,
}
impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            upstreams: vec![DnsUpstream {
                id: "system".into(),
                protocol: DnsProtocol::System,
                addresses: vec![],
                server_name: None,
                https_path: None,
                ca_certificate: None,
                proxy_id: None,
                proxy_bootstrap_ips: vec![],
            }],
            routes: vec![],
            default_upstream: "system".into(),
            listen: None,
            capture: None,
            timeout_ms: 5000,
            max_parallel: 64,
            cache_entries: 2048,
            cache_bytes: 16 * 1024 * 1024,
        }
    }
}
impl DnsConfig {
    pub fn validate(&self, proxy: &ProxyConfig) -> Result<(), ConfigError> {
        let invalid = |s: &str| ConfigError::Invalid(format!("dns: {s}"));
        if self.upstreams.is_empty()
            || self.upstreams.len() > 64
            || self.routes.len() > 1024
            || !(100..=60000).contains(&self.timeout_ms)
            || !(1..=256).contains(&self.max_parallel)
            || self.cache_entries > 100000
            || self.cache_bytes > 256 * 1024 * 1024
            || (self.cache_entries > 0 && self.cache_bytes < 65536)
        {
            return Err(invalid(
                "invalid upstream, route, timeout, concurrency or cache limits",
            ));
        }
        let mut ids = HashSet::new();
        if let Some(capture) = &self.capture
            && (!self.enabled
                || capture.object.as_os_str().is_empty()
                || capture.object.as_os_str().len() > 4096
                || !capture.cgroup.is_absolute()
                || capture.cgroup.as_os_str().len() > 4096
                || capture.port == 0
                || capture.port == 53
                || self.listen.is_some_and(|a| a.port() == capture.port)
                || proxy
                    .inbounds
                    .iter()
                    .any(|p| p.enabled && p.listen.port() == capture.port))
        {
            return Err(invalid(
                "capture needs enabled DNS, an object, absolute cgroup v2 path and a dedicated nonzero port different from 53 and all inlets",
            ));
        }
        let mut proxy_bootstrap = std::collections::HashMap::new();
        for up in &self.upstreams {
            if !crate::valid_id(&up.id) || !ids.insert(up.id.as_str()) {
                return Err(invalid("upstream IDs must be valid and unique"));
            }
            if up.addresses.len() > 8
                || up
                    .addresses
                    .iter()
                    .any(|a| a.port() == 0 || a.ip().is_unspecified() || a.ip().is_multicast())
                || up.proxy_bootstrap_ips.len() > 8
                || up
                    .proxy_bootstrap_ips
                    .iter()
                    .any(|ip| ip.is_unspecified() || ip.is_multicast())
            {
                return Err(invalid("invalid explicit bootstrap address"));
            }
            if up.protocol == DnsProtocol::System {
                if !up.addresses.is_empty()
                    || up.server_name.is_some()
                    || up.https_path.is_some()
                    || up.ca_certificate.is_some()
                    || up.proxy_id.is_some()
                    || !up.proxy_bootstrap_ips.is_empty()
                    || (self.enabled && (self.listen.is_some() || self.capture.is_some()))
                {
                    return Err(invalid(
                        "system resolver has no wire TTL or egress options and cannot serve a DNS listener",
                    ));
                }
                continue;
            }
            if up.addresses.is_empty() {
                return Err(invalid("wire upstreams need explicit IP socket addresses"));
            }
            if self
                .capture
                .as_ref()
                .is_some_and(|capture| up.addresses.iter().any(|a| a.port() == capture.port))
            {
                return Err(invalid(
                    "wire upstream ports must differ from the dedicated capture listener port",
                ));
            }
            let encrypted = matches!(up.protocol, DnsProtocol::Tls | DnsProtocol::Https);
            if encrypted {
                if up
                    .server_name
                    .as_ref()
                    .is_none_or(|s| !crate::valid_host(s))
                {
                    return Err(invalid(
                        "encrypted upstream needs a valid verified server_name",
                    ));
                }
            } else if up.server_name.is_some() || up.ca_certificate.is_some() {
                return Err(invalid("TLS options require DoT or DoH"));
            }
            if up.protocol == DnsProtocol::Https {
                let path = up.https_path.as_deref().unwrap_or("/dns-query");
                if !path.starts_with('/')
                    || path.starts_with("//")
                    || path.len() > 2048
                    || path.chars().any(|c| c.is_control() || c.is_whitespace())
                    || path.contains('#')
                {
                    return Err(invalid("invalid DoH path"));
                }
            } else if up.https_path.is_some() {
                return Err(invalid("https_path requires DoH"));
            }
            if let Some(id) = &up.proxy_id {
                if let Some(ips) = proxy_bootstrap.insert(id, &up.proxy_bootstrap_ips)
                    && ips != &up.proxy_bootstrap_ips
                {
                    return Err(invalid(
                        "a static proxy must have one consistent explicit bootstrap list",
                    ));
                }
                if up.protocol == DnsProtocol::Udp {
                    return Err(invalid(
                        "TCP-only HTTP/HTTPS/SOCKS5 DNS egress cannot carry UDP",
                    ));
                }
                let endpoint = proxy
                    .upstreams
                    .iter()
                    .find(|p| &p.id == id && p.enabled)
                    .ok_or_else(|| invalid("egress must reference an enabled static proxy"))?;
                if endpoint.host.parse::<IpAddr>().is_err() && up.proxy_bootstrap_ips.is_empty() {
                    return Err(invalid(
                        "a hostname proxy egress requires explicit proxy_bootstrap_ips to prevent a DNS loop",
                    ));
                }
            } else if !up.proxy_bootstrap_ips.is_empty() {
                return Err(invalid("proxy_bootstrap_ips requires proxy_id"));
            }
            if self.enabled
                && self.listen.is_some_and(|listen| {
                    up.addresses.iter().any(|a| {
                        *a == listen
                            || (listen.ip().is_unspecified()
                                && a.ip().is_loopback()
                                && a.port() == listen.port()
                                && a.is_ipv4() == listen.is_ipv4())
                    })
                })
            {
                return Err(invalid("DNS listener cannot be its own upstream"));
            }
        }
        if !ids.contains(self.default_upstream.as_str()) {
            return Err(invalid("unknown default upstream"));
        }
        let mut route_ids = HashSet::new();
        for route in &self.routes {
            if !crate::valid_id(&route.id)
                || !route_ids.insert(&route.id)
                || !ids.contains(route.upstream.as_str())
                || route.domain.as_ref().is_some_and(|d| {
                    !crate::valid_host(d.trim_end_matches('.')) || d.parse::<IpAddr>().is_ok()
                })
                || route.query_types.len() > 32
                || route.query_types.iter().any(|t| !valid_query_type(t))
                || route.purposes.len() > 4
            {
                return Err(invalid(
                    "invalid route ID, domain, query type, purpose or upstream reference",
                ));
            }
        }
        Ok(())
    }
}
pub fn valid_query_type(s: &str) -> bool {
    matches!(
        s,
        "A" | "AAAA"
            | "CNAME"
            | "MX"
            | "NS"
            | "SOA"
            | "TXT"
            | "SRV"
            | "PTR"
            | "CAA"
            | "HTTPS"
            | "SVCB"
            | "NAPTR"
            | "DS"
            | "DNSKEY"
            | "RRSIG"
            | "NSEC"
            | "NSEC3"
            | "TLSA"
    ) || s
        .strip_prefix("TYPE")
        .and_then(|n| n.parse::<u16>().ok())
        .is_some_and(|n| !matches!(n, 0 | 41 | 249..=255))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsCaptureConfig {
    pub object: PathBuf,
    pub cgroup: PathBuf,
    #[serde(default = "capture_port")]
    pub port: u16,
}
const fn capture_port() -> u16 {
    1053
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsSave {
    pub configuration: DnsConfig,
    pub expected_revision: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn listener_and_egress_cannot_recursively_bootstrap() {
        let mut config = DnsConfig {
            listen: Some("127.0.0.1:1053".parse().unwrap()),
            ..Default::default()
        };
        assert!(config.validate(&ProxyConfig::default()).is_err());
        config.upstreams[0].protocol = DnsProtocol::Udp;
        config.upstreams[0].addresses = vec!["127.0.0.1:1053".parse().unwrap()];
        assert!(config.validate(&ProxyConfig::default()).is_err());
        config.upstreams[0].addresses = vec!["1.1.1.1:53".parse().unwrap()];
        assert!(config.validate(&ProxyConfig::default()).is_ok());
        config.listen = Some("0.0.0.0:53".parse().unwrap());
        assert!(config.validate(&ProxyConfig::default()).is_ok());
        config.capture = Some(DnsCaptureConfig {
            object: "object".into(),
            cgroup: "/sys/fs/cgroup".into(),
            port: 1053,
        });
        assert!(config.validate(&ProxyConfig::default()).is_ok());
        config.upstreams[0].proxy_id = Some("missing".into());
        assert!(config.validate(&ProxyConfig::default()).is_err());
        assert!(!valid_query_type("AXFR"));
        assert!(!valid_query_type("TYPE252"));
    }
}
