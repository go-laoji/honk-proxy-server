//! Vendor contracts and bounded fetching. No task scheduling or implicit retries.
use base64::Engine;
use hmac::{Hmac, Mac};
use hps_config::{
    FetchMethod, ProviderAuth, ProviderConfig, ProviderKind, UpstreamConfig, valid_host,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    net::{IpAddr, SocketAddr},
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseCandidate {
    pub upstream: UpstreamConfig,
    pub expires_at: i64,
    pub expiry_basis: String,
    pub region: Option<String>,
}
#[derive(Debug)]
pub struct FetchResult {
    pub nodes: Vec<LeaseCandidate>,
    pub rejected: usize,
    pub quota: Option<u64>,
}
#[derive(Debug, Clone, thiserror::Error)]
#[error("{code}")]
pub struct FetchError {
    pub code: &'static str,
    pub outcome_unknown: bool,
    pub retry_after_secs: Option<u64>,
}
impl FetchError {
    fn known(code: &'static str) -> Self {
        Self {
            code,
            outcome_unknown: false,
            retry_after_secs: None,
        }
    }
    fn unknown(code: &'static str) -> Self {
        Self {
            code,
            outcome_unknown: true,
            retry_after_secs: None,
        }
    }
}
pub fn fingerprint(config: &ProviderConfig) -> String {
    // Do not publish the fingerprint input or use raw credentials as identifiers.
    let mut provenance = config.clone();
    provenance.name.clear();
    provenance.enabled = false;
    provenance.count = 1;
    provenance.min_interval_secs = 1;
    provenance.min_remaining_secs = 0;
    provenance.mode = hps_config::LeaseMode::AppendLeases;
    provenance.full_snapshot = false;
    provenance.minimum_snapshot_percent = 0;
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&provenance).expect("provider serialization"))
    )
}
pub fn sign(method: &str, path: &str, params: &BTreeMap<String, String>, key: &str) -> String {
    let raw = format!(
        "{method}{path}?{}",
        params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&")
    );
    let mut hmac =
        Hmac::<Sha1>::new_from_slice(key.as_bytes()).expect("HMAC accepts all key lengths");
    hmac.update(raw.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hmac.finalize().into_bytes())
}
fn encode(value: &str) -> String {
    let mut result = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            result.push(b as char);
        } else {
            result.push_str(&format!("%{b:02X}"));
        }
    }
    result
}
pub struct ProviderClient {
    client: reqwest::Client,
}
impl ProviderClient {
    pub fn new(config: &ProviderConfig) -> Result<Self, FetchError> {
        config
            .validate()
            .map_err(|_| FetchError::known("invalid_provider"))?;
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_millis(config.timeout_ms))
            .connect_timeout(Duration::from_millis(config.timeout_ms.min(10000)))
            .user_agent("honk-proxy-server/0.1");
        if let Some(proxy) = &config.fetch_proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy).map_err(|_| FetchError::known("invalid_fetch_proxy"))?,
            );
        }
        Ok(Self {
            client: builder
                .build()
                .map_err(|_| FetchError::known("invalid_fetch_client"))?,
        })
    }
    async fn request(
        &self,
        config: &ProviderConfig,
        path: Option<&str>,
        mut params: BTreeMap<String, String>,
        now: i64,
    ) -> Result<ValueOrBytes, FetchError> {
        let mut url = reqwest::Url::parse(&config.endpoint)
            .map_err(|_| FetchError::known("invalid_endpoint"))?;
        if let Some(path) = path {
            if path == "/api/getipwhitelist" && url.host_str() == Some("dps.kdlapi.com") {
                url.set_host(Some("dev.kdlapi.com"))
                    .map_err(|_| FetchError::known("invalid_endpoint"))?;
            }
            url.set_path(path);
        }
        let method = if path.is_some() || config.method == FetchMethod::Get {
            "GET"
        } else {
            "POST"
        };
        match &config.auth {
            ProviderAuth::None => {}
            ProviderAuth::Token { secret_id, token } => {
                params.insert("secret_id".into(), secret_id.clone());
                params.insert("sign_type".into(), "token".into());
                params.insert("signature".into(), token.clone());
            }
            ProviderAuth::HmacSha1 {
                secret_id,
                secret_key,
            } => {
                params.insert("secret_id".into(), secret_id.clone());
                params.insert("sign_type".into(), "hmacsha1".into());
                params.insert("timestamp".into(), now.to_string());
                let nonce = u64::from_be_bytes(
                    uuid::Uuid::new_v4().as_bytes()[..8]
                        .try_into()
                        .expect("UUID bytes"),
                ) >> 1;
                params.insert("nonce".into(), nonce.max(1).to_string());
                params.remove("signature");
                let signature = sign(method, url.path(), &params, secret_key);
                params.insert("signature".into(), signature);
            }
        }
        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let mut request = if method == "GET" {
            url.set_query(Some(&query));
            self.client.get(url)
        } else {
            self.client
                .post(url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(query)
        };
        for (key, value) in &config.headers {
            request = request.header(key, value);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| FetchError::unknown("transport_outcome_unknown"))?;
        let status = response.status().as_u16();
        if status != 200 {
            // A non-success HTTP response does not prove that extraction was not billed.
            let mut error = FetchError::unknown(match status {
                401 | 403 => "http_auth_or_whitelist",
                429 => "rate_limited",
                300..=399 => "redirect_rejected",
                _ => "vendor_http_error",
            });
            error.retry_after_secs = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(|s| s.min(86400));
            return Err(error);
        }
        if response
            .content_length()
            .is_some_and(|n| n > config.max_response_bytes as u64)
        {
            return Err(FetchError::unknown("response_too_large"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| FetchError::unknown("response_outcome_unknown"))?
        {
            if bytes.len() + chunk.len() > config.max_response_bytes {
                return Err(FetchError::unknown("response_too_large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(ValueOrBytes(bytes))
    }
    pub async fn fetch(
        &self,
        config: &ProviderConfig,
        started_at: i64,
    ) -> Result<FetchResult, FetchError> {
        let mut params = config.params.clone();
        if config.kind == ProviderKind::Kuaidaili {
            params.insert("format".into(), "json".into());
            params.insert("num".into(), config.count.to_string());
        }
        let bytes = self.request(config, None, params, started_at).await?.0;
        let mut result = parse(config, &bytes, started_at)?;
        if config.query_valid_time {
            let proxies = result
                .nodes
                .iter()
                .map(|n| authority(&n.upstream.host, n.upstream.port))
                .collect::<Vec<_>>()
                .join(",");
            let time_started = unix_now();
            let bytes = self
                .request(
                    config,
                    Some("/api/getdpsvalidtime"),
                    BTreeMap::from([("proxy".into(), proxies)]),
                    time_started,
                )
                .await
                .map_err(|_| FetchError::unknown("lease_time_lookup_failed"))?
                .0;
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| FetchError::unknown("invalid_lease_time_response"))?;
            kdl_success(&value).map_err(|_| FetchError::unknown("lease_time_lookup_rejected"))?;
            let data = value
                .get("data")
                .and_then(Value::as_object)
                .ok_or_else(|| FetchError::unknown("invalid_lease_time_response"))?;
            for node in &mut result.nodes {
                let ttl = data
                    .get(&authority(&node.upstream.host, node.upstream.port))
                    .and_then(Value::as_u64)
                    .filter(|ttl| *ttl > 0 && *ttl <= 31_536_000)
                    .ok_or_else(|| FetchError::unknown("missing_lease_time"))?;
                let deadline = time_started.saturating_add(ttl as i64).min(node.expires_at);
                node.expires_at = deadline;
                node.upstream.expires_at_unix = Some(deadline.max(0) as u64);
                node.expiry_basis = "vendor_remaining_seconds_from_lookup_start".into();
            }
        }
        Ok(result)
    }
    /// These endpoints only read vendor state; never change whitelist, orders or purchases.
    pub async fn diagnostic(
        &self,
        config: &ProviderConfig,
        kind: &str,
    ) -> Result<Value, FetchError> {
        if config.kind != ProviderKind::Kuaidaili {
            return Ok(
                serde_json::json!({"supported":false,"state":"unknown","reason":"source_has_no_diagnostic_contract"}),
            );
        }
        let path = match kind {
            "quota" => "/api/getipbalance",
            "whitelist" => "/api/getipwhitelist",
            _ => return Err(FetchError::known("unsupported_diagnostic")),
        };
        let bytes = self
            .request(config, Some(path), BTreeMap::new(), unix_now())
            .await?
            .0;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| FetchError::known("invalid_diagnostic_response"))?;
        kdl_success(&value)?;
        // Only forward documented non-credential fields, never raw vendor bodies.
        Ok(if kind == "quota" {
            serde_json::json!({"supported":true,"state":"known","unit":"ips","balance":value.pointer("/data/balance").and_then(Value::as_u64)})
        } else {
            serde_json::json!({"supported":true,"state":"known","whitelist":value.pointer("/data/ipwhitelist").and_then(Value::as_array).map(|items|items.iter().filter_map(Value::as_str).filter(|v|v.parse::<IpAddr>().is_ok()).take(1000).collect::<Vec<_>>())})
        })
    }
}
struct ValueOrBytes(Vec<u8>);
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
fn kdl_success(value: &Value) -> Result<(), FetchError> {
    match value.get("code").and_then(Value::as_i64) {
        Some(0) => Ok(()),
        Some(1 | 2) => Err(FetchError::known("quota_exhausted")),
        Some(3) => Err(FetchError::known("empty_result")),
        Some(-6) => Err(FetchError::known("whitelist_rejected")),
        Some(-51) => Err(FetchError::known("vendor_source_limit")),
        Some(-15 | -13 | -12 | -11 | -16 | -14 | -2) => Err(FetchError::known("order_unavailable")),
        Some(-100..=-1) => Err(FetchError::known("vendor_auth_or_parameters")),
        _ => Err(FetchError::unknown("invalid_vendor_envelope")),
    }
}
pub fn parse(
    config: &ProviderConfig,
    bytes: &[u8],
    started_at: i64,
) -> Result<FetchResult, FetchError> {
    if bytes.len() > config.max_response_bytes {
        return Err(FetchError::unknown("response_too_large"));
    }
    let invalid = || FetchError::unknown("invalid_vendor_payload");
    type ParsedEndpoint = (String, u16, Option<u64>, Option<i64>, Option<String>);
    let mut items: Vec<ParsedEndpoint> = vec![];
    let mut quota = None;
    match config.kind {
        ProviderKind::Kuaidaili => {
            let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
            kdl_success(&value)?;
            let list = value
                .pointer("/data/proxy_list")
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            if list.len() > config.count as usize
                || value.pointer("/data/count").and_then(Value::as_u64) != Some(list.len() as u64)
            {
                return Err(invalid());
            }
            for item in list {
                let (host, port) = parse_authority(item.as_str().ok_or_else(invalid)?)?;
                items.push((host, port, None, None, None));
            }
            quota = value
                .pointer("/data/order_left_count")
                .or_else(|| value.pointer("/data/today_left_count"))
                .and_then(Value::as_u64);
        }
        ProviderKind::Json => {
            let value: Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
            let list = value
                .pointer(&config.json.list)
                .and_then(Value::as_array)
                .ok_or_else(invalid)?;
            if list.len() > config.count as usize {
                return Err(invalid());
            }
            for item in list {
                let host = item
                    .pointer(&config.json.host)
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?
                    .trim()
                    .to_owned();
                let port = item
                    .pointer(&config.json.port)
                    .and_then(integer)
                    .and_then(|v| u16::try_from(v).ok())
                    .filter(|v| *v > 0)
                    .ok_or_else(invalid)?;
                let ttl = config
                    .json
                    .ttl
                    .as_ref()
                    .and_then(|p| item.pointer(p))
                    .map(|v| integer(v).ok_or_else(invalid))
                    .transpose()?;
                let expiry = config
                    .json
                    .expires_at
                    .as_ref()
                    .and_then(|p| item.pointer(p))
                    .map(|v| {
                        integer(v)
                            .and_then(|n| {
                                i64::try_from(if config.json.expiry_milliseconds {
                                    n / 1000
                                } else {
                                    n
                                })
                                .ok()
                            })
                            .ok_or_else(invalid)
                    })
                    .transpose()?;
                items.push((host, port, ttl, expiry, None));
            }
        }
        ProviderKind::Text => {
            for line in std::str::from_utf8(bytes)
                .map_err(|_| invalid())?
                .lines()
                .filter(|v| !v.trim().is_empty())
            {
                let (host, port) = parse_authority(line.trim())?;
                items.push((host, port, None, None, None));
                if items.len() > config.count as usize {
                    return Err(invalid());
                }
            }
        }
        ProviderKind::YunFree => {
            let text = match std::str::from_utf8(bytes) {
                Ok(text) => std::borrow::Cow::Borrowed(text),
                Err(_) => encoding_rs::GBK.decode(bytes).0,
            };
            let html = scraper::Html::parse_document(&text);
            let rows = scraper::Selector::parse("#list table tbody tr, #list table tr")
                .expect("static selector");
            let cells = scraper::Selector::parse("td").expect("static selector");
            for row in html.select(&rows) {
                let values = row
                    .select(&cells)
                    .map(|c| c.text().collect::<String>().trim().to_owned())
                    .collect::<Vec<_>>();
                if values.len() < 4 {
                    continue;
                }
                let Ok(ip) = values[0].parse::<IpAddr>() else {
                    continue;
                };
                let Ok(port) = values[1].parse::<u16>() else {
                    continue;
                };
                // HTTPS here describes target support through HTTP CONNECT, not TLS to the proxy.
                if port == 0
                    || !["HTTP", "HTTPS"].contains(&values[3].to_ascii_uppercase().as_str())
                {
                    continue;
                }
                items.push((ip.to_string(), port, None, None, values.get(4).cloned()));
                if items.len() >= config.count as usize {
                    break;
                }
            }
        }
    }
    if items.is_empty() {
        return Err(FetchError::known("empty_result"));
    }
    let mut nodes = vec![];
    let mut rejected = 0;
    let mut seen = HashSet::new();
    for (host, port, ttl, expiry, region) in items {
        if !valid_host(&host) || !seen.insert((host.clone(), port)) {
            rejected += 1;
            continue;
        }
        let ttl = ttl.or(config.ttl_secs);
        let deadline = match (ttl, expiry) {
            (Some(ttl), expiry) if ttl > 0 && ttl <= 31_536_000 => started_at
                .saturating_add(ttl as i64)
                .min(expiry.unwrap_or(i64::MAX)),
            (None, Some(expiry)) => expiry,
            (None, None) if config.query_valid_time => started_at.saturating_add(31_536_000),
            _ => {
                rejected += 1;
                continue;
            }
        };
        if deadline <= started_at.saturating_add(config.min_remaining_secs as i64) {
            rejected += 1;
            continue;
        }
        let basis = if config.kind == ProviderKind::YunFree {
            "local_cache_upper_bound_not_vendor_guarantee"
        } else if expiry.is_some() {
            "vendor_absolute_unix"
        } else if ttl.is_some() {
            "ttl_seconds_from_request_start"
        } else {
            "pending_vendor_lifetime_lookup"
        };
        nodes.push(LeaseCandidate {
            upstream: UpstreamConfig {
                id: String::new(),
                protocol: config.protocol,
                host,
                port,
                enabled: true,
                credentials: config.upstream_credentials.clone(),
                expires_at_unix: Some(deadline.max(0) as u64),
                ca_certificate: None,
                tls_server_name: None,
            },
            expires_at: deadline,
            expiry_basis: basis.into(),
            region,
        });
    }
    if nodes.is_empty() {
        return Err(FetchError::known("no_admissible_leases"));
    }
    Ok(FetchResult {
        nodes,
        rejected,
        quota,
    })
}
fn integer(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}
fn parse_authority(value: &str) -> Result<(String, u16), FetchError> {
    let result = if let Ok(addr) = value.parse::<SocketAddr>() {
        Some((addr.ip().to_string(), addr.port()))
    } else {
        value
            .rsplit_once(':')
            .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host.to_owned(), port)))
    };
    result
        .filter(|(host, port)| *port > 0 && valid_host(host))
        .ok_or_else(|| FetchError::unknown("invalid_proxy_endpoint"))
}

#[cfg(test)]
mod tests;
