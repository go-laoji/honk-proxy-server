use crate::{ConfigError, ProxyProtocol, UpstreamCredentials, valid_host};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Kuaidaili,
    YunFree,
    #[default]
    Json,
    Text,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseMode {
    #[default]
    AppendLeases,
    ReplaceSnapshot,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum FetchMethod {
    #[default]
    Get,
    Post,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderAuth {
    #[default]
    None,
    Token {
        secret_id: String,
        token: String,
    },
    HmacSha1 {
        secret_id: String,
        secret_key: String,
    },
}
impl std::fmt::Debug for ProviderAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderAuth([REDACTED])")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JsonFields {
    pub list: String,
    pub host: String,
    pub port: String,
    pub ttl: Option<String>,
    pub expires_at: Option<String>,
    pub expiry_milliseconds: bool,
}
impl Default for JsonFields {
    fn default() -> Self {
        Self {
            list: "/data/proxies".into(),
            host: "/ip".into(),
            port: "/port".into(),
            ttl: Some("/ttl".into()),
            expires_at: None,
            expiry_milliseconds: false,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub kind: ProviderKind,
    /// Endpoint never contains credentials or query parameters. Use params/headers/auth.
    pub endpoint: String,
    pub method: FetchMethod,
    pub params: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub auth: ProviderAuth,
    pub protocol: ProxyProtocol,
    pub count: u32,
    pub ttl_secs: Option<u64>,
    pub query_valid_time: bool,
    pub min_remaining_secs: u64,
    pub mode: LeaseMode,
    pub full_snapshot: bool,
    pub minimum_snapshot_percent: u8,
    pub min_interval_secs: u64,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
    pub fetch_proxy: Option<String>,
    pub upstream_credentials: Option<UpstreamCredentials>,
    pub json: JsonFields,
}
impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: false,
            kind: ProviderKind::Json,
            endpoint: String::new(),
            method: FetchMethod::Get,
            params: BTreeMap::new(),
            headers: BTreeMap::new(),
            auth: ProviderAuth::None,
            protocol: ProxyProtocol::Http,
            count: 10,
            ttl_secs: None,
            query_valid_time: false,
            min_remaining_secs: 30,
            mode: LeaseMode::AppendLeases,
            full_snapshot: false,
            minimum_snapshot_percent: 50,
            min_interval_secs: 60,
            timeout_ms: 10000,
            max_response_bytes: 2 * 1024 * 1024,
            fetch_proxy: None,
            upstream_credentials: None,
            json: JsonFields::default(),
        }
    }
}
impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Endpoints, headers and generic parameters may all contain vendor secrets.
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}
impl ProviderConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = || {
            ConfigError::Invalid("invalid provider configuration; check ID, endpoint, lease, limits and authentication fields".into())
        };
        let url = url::Url::parse(&self.endpoint).map_err(|_| invalid())?;
        if !super::proxy::valid_id(&self.id)
            || self.name.len() > 128
            || !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !(1..=1000).contains(&self.count)
            || !(100..=120000).contains(&self.timeout_ms)
            || !(1024..=4 * 1024 * 1024).contains(&self.max_response_bytes)
            || !(1..=86400).contains(&self.min_interval_secs)
            || self.min_remaining_secs > 86400
            || self
                .ttl_secs
                .is_some_and(|ttl| ttl == 0 || ttl > 31_536_000)
            || self.minimum_snapshot_percent > 100
            || (self.mode == LeaseMode::ReplaceSnapshot && !self.full_snapshot)
            || self.params.len() > 64
            || self.headers.len() > 32
            || self
                .params
                .iter()
                .any(|(k, v)| k.is_empty() || k.len() > 128 || v.len() > 4096)
            || self.headers.iter().any(|(k, v)| {
                k.is_empty()
                    || k.len() > 128
                    || v.len() > 4096
                    || k.contains(['\r', '\n'])
                    || v.contains(['\r', '\n'])
            })
        {
            return Err(invalid());
        }
        if self.kind == ProviderKind::Kuaidaili
            && (url.scheme() != "https"
                || self.method != FetchMethod::Get
                || matches!(self.auth, ProviderAuth::None)
                || (!self.query_valid_time && self.ttl_secs.is_none())
                || self
                    .params
                    .keys()
                    .any(|k| !["area", "area_ex", "carrier", "dedup"].contains(&k.as_str()))
                || self.protocol == ProxyProtocol::Https)
        {
            return Err(invalid());
        }
        if matches!(self.kind, ProviderKind::Text | ProviderKind::YunFree)
            && self.ttl_secs.is_none()
        {
            return Err(invalid());
        }
        if self.query_valid_time && self.kind != ProviderKind::Kuaidaili {
            return Err(invalid());
        }
        if self.kind == ProviderKind::YunFree
            && (self.method != FetchMethod::Get
                || self.protocol != ProxyProtocol::Http
                || !matches!(self.auth, ProviderAuth::None))
        {
            return Err(invalid());
        }
        let credentials = match &self.auth {
            ProviderAuth::None => None,
            ProviderAuth::Token { secret_id, token } => Some((secret_id, token)),
            ProviderAuth::HmacSha1 {
                secret_id,
                secret_key,
            } => Some((secret_id, secret_key)),
        };
        if credentials.is_some_and(|(id, key)| {
            id.is_empty() || id.len() > 255 || key.is_empty() || key.len() > 4096
        }) {
            return Err(invalid());
        }
        if let Some(proxy) = &self.fetch_proxy
            && proxy != "[REDACTED]"
        {
            let url = url::Url::parse(proxy).map_err(|_| invalid())?;
            if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
                || url.host_str().is_none()
            {
                return Err(invalid());
            }
        }
        if let Some(c) = &self.upstream_credentials
            && (c.username.is_empty()
                || c.username.contains(':')
                || c.username.len() > 255
                || c.password.len() > 255
                || (self.protocol == ProxyProtocol::Socks5 && c.password.is_empty()))
        {
            return Err(invalid());
        }
        for pointer in [&self.json.list, &self.json.host, &self.json.port]
            .into_iter()
            .chain(self.json.ttl.iter())
            .chain(self.json.expires_at.iter())
        {
            if !pointer.starts_with('/') || pointer.len() > 256 {
                return Err(invalid());
            }
        }
        // Prevent accidental empty/invalid hosts hidden inside a URL parse edge case.
        if !valid_host(url.host_str().unwrap_or_default()) {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn redacted(&self) -> Self {
        let mut result = self.clone();
        match &mut result.auth {
            ProviderAuth::None => {}
            ProviderAuth::Token { secret_id, token } => {
                *secret_id = "[REDACTED]".into();
                *token = "[REDACTED]".into();
            }
            ProviderAuth::HmacSha1 {
                secret_id,
                secret_key,
            } => {
                *secret_id = "[REDACTED]".into();
                *secret_key = "[REDACTED]".into();
            }
        }
        for value in result.headers.values_mut() {
            *value = "[REDACTED]".into();
        }
        for (key, value) in &mut result.params {
            if sensitive_parameter(key) {
                *value = "[REDACTED]".into();
            }
        }
        if let Some(proxy) = &mut result.fetch_proxy {
            *proxy = "[REDACTED]".into();
        }
        if let Some(c) = &mut result.upstream_credentials {
            c.username = "[REDACTED]".into();
            c.password = "[REDACTED]".into();
        }
        result
    }
    pub fn restore_redacted(&mut self, old: Option<&Self>) -> Result<(), ConfigError> {
        fn restore(value: &mut String, old: Option<&String>) -> Result<(), ConfigError> {
            if value == "[REDACTED]" {
                *value = old
                    .ok_or_else(|| {
                        ConfigError::Invalid(
                            "redacted provider credential has no existing reference".into(),
                        )
                    })?
                    .clone();
            }
            Ok(())
        }
        match &mut self.auth {
            ProviderAuth::None => {}
            ProviderAuth::Token { secret_id, token } => {
                let pair = old.and_then(|o| {
                    if let ProviderAuth::Token { secret_id, token } = &o.auth {
                        Some((secret_id, token))
                    } else {
                        None
                    }
                });
                restore(secret_id, pair.map(|p| p.0))?;
                restore(token, pair.map(|p| p.1))?;
            }
            ProviderAuth::HmacSha1 {
                secret_id,
                secret_key,
            } => {
                let pair = old.and_then(|o| {
                    if let ProviderAuth::HmacSha1 {
                        secret_id,
                        secret_key,
                    } = &o.auth
                    {
                        Some((secret_id, secret_key))
                    } else {
                        None
                    }
                });
                restore(secret_id, pair.map(|p| p.0))?;
                restore(secret_key, pair.map(|p| p.1))?;
            }
        }
        for (k, v) in &mut self.params {
            restore(v, old.and_then(|o| o.params.get(k)))?;
        }
        for (k, v) in &mut self.headers {
            restore(v, old.and_then(|o| o.headers.get(k)))?;
        }
        if let Some(v) = &mut self.fetch_proxy {
            restore(v, old.and_then(|o| o.fetch_proxy.as_ref()))?;
        }
        if let Some(c) = &mut self.upstream_credentials {
            restore(
                &mut c.username,
                old.and_then(|o| o.upstream_credentials.as_ref())
                    .map(|c| &c.username),
            )?;
            restore(
                &mut c.password,
                old.and_then(|o| o.upstream_credentials.as_ref())
                    .map(|c| &c.password),
            )?;
        }
        self.validate()
    }
}
fn sensitive_parameter(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "key",
        "token",
        "secret",
        "signature",
        "password",
        "auth",
        "credential",
    ]
    .iter()
    .any(|part| key.contains(part))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSave {
    pub configuration: ProviderConfig,
    pub expected_revision: i64,
}
