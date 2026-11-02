use crate::ConfigError;
pub use hps_domain::ResourceLimits;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, net::SocketAddr, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyProtocol {
    Http,
    Https,
    Socks5,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyConfig {
    pub inbounds: Vec<InboundConfig>,
    pub upstreams: Vec<UpstreamConfig>,
    pub limits: ResourceLimits,
    pub selection: crate::SelectionConfig,
    pub routing: crate::RoutingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboundConfig {
    pub id: String,
    pub protocol: ProxyProtocol,
    pub listen: SocketAddr,
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub tls: Option<ServerTls>,
    #[serde(default = "entry_connections")]
    pub max_connections: usize,
    #[serde(default)]
    pub bytes_per_second: u64,
}

const fn enabled() -> bool {
    true
}
const fn entry_connections() -> usize {
    256
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerTls {
    pub certificate: PathBuf,
    pub private_key: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamConfig {
    pub id: String,
    pub protocol: ProxyProtocol,
    pub host: String,
    pub port: u16,
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub credentials: Option<UpstreamCredentials>,
    pub expires_at_unix: Option<u64>,
    pub ca_certificate: Option<PathBuf>,
    pub tls_server_name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamCredentials {
    pub username: String,
    pub password: String,
}
impl std::fmt::Debug for UpstreamCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UpstreamCredentials([REDACTED])")
    }
}

impl ProxyConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: &str| ConfigError::Invalid(message.into());
        if self.inbounds.len() > 64 || self.upstreams.len() > 100_000 {
            return Err(invalid(
                "proxy configuration exceeds the entry/node count limit",
            ));
        }
        let mut ids = HashSet::new();
        let mut addresses = HashSet::new();
        for inbound in &self.inbounds {
            if !valid_id(&inbound.id) || !ids.insert(&inbound.id) {
                return Err(invalid(
                    "inbound IDs must be unique ASCII letters/digits/_/- (1–64 bytes)",
                ));
            }
            if inbound.enabled && inbound.listen.port() != 0 && !addresses.insert(inbound.listen) {
                return Err(invalid("enabled inbounds cannot share a listen address"));
            }
            if (inbound.protocol == ProxyProtocol::Https) != inbound.tls.is_some() {
                return Err(invalid(
                    "only HTTPS inbounds require TLS certificate and private key paths",
                ));
            }
            if !(1..=1_000_000).contains(&inbound.max_connections) {
                return Err(invalid("inbound max_connections must be 1–1000000"));
            }
        }
        ids.clear();
        for upstream in &self.upstreams {
            if !valid_id(&upstream.id)
                || !ids.insert(&upstream.id)
                || upstream.port == 0
                || !valid_host(&upstream.host)
            {
                return Err(invalid(
                    "upstreams require unique valid IDs, an ASCII hostname/IP and a nonzero port",
                ));
            }
            if upstream.protocol != ProxyProtocol::Https
                && (upstream.ca_certificate.is_some() || upstream.tls_server_name.is_some())
            {
                return Err(invalid("upstream TLS settings only apply to HTTPS proxies"));
            }
            if let Some(credentials) = &upstream.credentials
                && (credentials.username.is_empty()
                    || credentials.username.contains(':')
                    || credentials.username.len() > 255
                    || credentials.password.len() > 255
                    || (upstream.protocol == ProxyProtocol::Socks5
                        && credentials.password.is_empty()))
            {
                return Err(invalid(
                    "upstream credentials require a 1–255 byte username without colon and a password of at most 255 bytes",
                ));
            }
        }
        let limits = &self.limits;
        if !(1..=1_000_000).contains(&limits.max_connections)
            || !(1024..=1024 * 1024).contains(&limits.buffer_bytes)
            || !(8192..=1024 * 1024).contains(&limits.max_header_bytes)
            || [
                limits.handshake_timeout_ms,
                limits.connect_timeout_ms,
                limits.idle_timeout_ms,
            ]
            .iter()
            .any(|ms| !(1..=86_400_000).contains(ms))
            || limits.bytes_per_second > 1_000_000_000_000
            || self
                .inbounds
                .iter()
                .any(|i| i.bytes_per_second > 1_000_000_000_000)
        {
            return Err(invalid("proxy resource limits are out of range"));
        }
        self.selection.validate(self)?;
        self.routing.validate(self)?;
        Ok(())
    }
}

/// Parse one manual proxy address without resolving, probing, or echoing credentials in errors.
pub fn parse_upstream_address(
    value: &str,
    default: ProxyProtocol,
) -> Result<UpstreamConfig, ConfigError> {
    let invalid = || ConfigError::Invalid("invalid manual proxy address or credentials".into());
    if value.len() > 4096 {
        return Err(invalid());
    }
    let scheme = match default {
        ProxyProtocol::Http => "http",
        ProxyProtocol::Https => "https",
        ProxyProtocol::Socks5 => "socks5",
    };
    let address = if value.contains("://") {
        value.to_owned()
    } else {
        format!("{scheme}://{value}")
    };
    let url = url::Url::parse(&address).map_err(|_| invalid())?;
    let protocol = match url.scheme() {
        "http" => ProxyProtocol::Http,
        "https" => ProxyProtocol::Https,
        "socks5" | "socks5h" => ProxyProtocol::Socks5,
        _ => return Err(invalid()),
    };
    if !["", "/"].contains(&url.path()) || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid());
    }
    fn decode(s: &str) -> Option<String> {
        let mut bytes = Vec::with_capacity(s.len());
        let mut input = s.as_bytes().iter().copied();
        while let Some(b) = input.next() {
            if b == b'%' {
                let a = (input.next()? as char).to_digit(16)?;
                let b = (input.next()? as char).to_digit(16)?;
                bytes.push((a * 16 + b) as u8);
            } else {
                bytes.push(b);
            }
        }
        String::from_utf8(bytes).ok()
    }
    let credentials = if !url.username().is_empty() || url.password().is_some() {
        Some(UpstreamCredentials {
            username: decode(url.username()).ok_or_else(invalid)?,
            password: decode(url.password().unwrap_or_default()).ok_or_else(invalid)?,
        })
    } else {
        None
    };
    let node = UpstreamConfig {
        id: "preview".into(),
        protocol,
        host: url
            .host_str()
            .ok_or_else(invalid)?
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_owned(),
        port: url.port_or_known_default().unwrap_or(1080),
        enabled: true,
        credentials,
        expires_at_unix: None,
        ca_certificate: None,
        tls_server_name: None,
    };
    ProxyConfig {
        upstreams: vec![node.clone()],
        ..Default::default()
    }
    .validate()
    .map_err(|_| invalid())?;
    Ok(node)
}

pub fn valid_host(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    host.parse::<std::net::IpAddr>().is_ok()
        || (!host.is_empty()
            && host.len() <= 253
            && host.split('.').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }))
}
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_redacted_and_invalid_config_never_quotes_input() {
        let body = "[[proxy.upstreams]]\nid='test'\nprotocol='socks5'\nhost='127.0.0.1'\nport=1080\n[proxy.upstreams.credentials]\nusername='supplier'\npassword='private-password'\n";
        let config = crate::DaemonConfig::from_toml(body).unwrap();
        assert!(!format!("{config:?}").contains("private-password"));
        assert!(
            !toml::to_string(&config.redacted())
                .unwrap()
                .contains("private-password")
        );
        let error = crate::DaemonConfig::from_toml(&body.replace(
            "password='private-password'",
            "password=['private-password']",
        ))
        .unwrap_err();
        assert!(!error.to_string().contains("private-password"));
        assert!(valid_host("::1"));
        assert!(!valid_host("user:password@host"));
    }
}
