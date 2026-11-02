//! Bounded standard subscription parser/fetcher, separate from extraction APIs.
use super::{FetchError, ProviderResolver};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
};
use hps_config::{
    ProxyProtocol, SubscriptionConfig, SubscriptionFormat, UpstreamConfig, UpstreamCredentials,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::Arc, time::Duration};
const MAX_BYTES: usize = 1024 * 1024;
pub fn fingerprint(config: &SubscriptionConfig) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(config.id.as_str(), config.url.as_str(), config.format))
                .expect("subscription fingerprint")
        )
    )
}
pub fn parse(
    config: &SubscriptionConfig,
    document: &str,
) -> Result<Vec<UpstreamConfig>, FetchError> {
    config
        .validate()
        .map_err(|_| FetchError::known("invalid_subscription"))?;
    if document.len() > MAX_BYTES || document.trim().is_empty() {
        return Err(FetchError::known("empty_or_oversized_subscription"));
    }
    let mut nodes = match config.format {
        SubscriptionFormat::Uri => parse_uris(document)?,
        SubscriptionFormat::Clash => parse_clash(document)?,
    };
    if nodes.is_empty() || nodes.len() > config.max_nodes {
        return Err(FetchError::known("subscription_node_budget"));
    }
    let mut seen = HashSet::new();
    for node in &mut nodes {
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(node).expect("node identity"))
        );
        node.id = format!("sub:{}:{}", config.id, &hash[..24]);
        if !seen.insert(node.id.clone()) {
            return Err(FetchError::known("duplicate_subscription_endpoint"));
        }
    }
    Ok(nodes)
}
fn parse_uris(document: &str) -> Result<Vec<UpstreamConfig>, FetchError> {
    let decoded;
    let body = if document.contains("://") {
        document
    } else {
        let compact = document.split_whitespace().collect::<String>();
        let bytes = [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD]
            .iter()
            .find_map(|e| e.decode(&compact).ok())
            .ok_or_else(|| FetchError::known("invalid_uri_subscription"))?;
        decoded =
            String::from_utf8(bytes).map_err(|_| FetchError::known("invalid_uri_subscription"))?;
        &decoded
    };
    if body.lines().count() > 10000 {
        return Err(FetchError::known("subscription_node_budget"));
    }
    body.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(|line| {
            if !line.contains("://") {
                return Err(FetchError::known("invalid_uri_subscription"));
            }
            // URI fragment is a display name only; it never changes endpoint identity.
            let address = line.split_once('#').map_or(line, |(a, _)| a);
            hps_config::parse_upstream_address(address, ProxyProtocol::Http)
                .map_err(|_| FetchError::known("unsupported_or_invalid_subscription_uri"))
        })
        .collect()
}
#[derive(Deserialize)]
struct ClashDocument {
    #[serde(deserialize_with = "bounded_proxies")]
    proxies: Vec<ClashProxy>,
}
fn bounded_proxies<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<ClashProxy>, D::Error> {
    struct Proxies;
    impl<'de> serde::de::Visitor<'de> for Proxies {
        type Value = Vec<ClashProxy>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("at most 10000 supported proxy entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut nodes = Vec::new();
            while let Some(node) = seq.next_element()? {
                if nodes.len() == 10000 {
                    return Err(serde::de::Error::custom(
                        "subscription node budget exceeded",
                    ));
                }
                nodes.push(node);
            }
            Ok(nodes)
        }
    }
    deserializer.deserialize_seq(Proxies)
}
// This deliberately restricted YAML subset has no anchors/aliases and at most 32
// indentation/flow levels. Scan before serde/libyaml can allocate an event tree.
// Quoted scalar contents and comments are never interpreted as references.
fn yaml_budget(document: &str) -> Result<(), FetchError> {
    let bytes = document.as_bytes();
    let mut i = 0;
    let mut quote = 0u8;
    let mut comment = false;
    let mut line_start = true;
    let mut indents = vec![0usize];
    let mut flow_depth = 0usize;
    fn boundary(byte: u8) -> bool {
        byte.is_ascii_whitespace() || b"[]{}:,-?".contains(&byte)
    }
    while i < bytes.len() {
        let byte = bytes[i];
        if line_start && quote == 0 {
            let mut end = i;
            while end < bytes.len() && bytes[end] == b' ' {
                end += 1;
            }
            if end < bytes.len() && !b"\r\n#".contains(&bytes[end]) {
                let indent = end - i;
                while indents.last().is_some_and(|n| *n > indent) {
                    indents.pop();
                }
                if indents.last().is_none_or(|n| *n < indent) {
                    indents.push(indent);
                }
                if indents.len() > 32 {
                    return Err(FetchError::known("subscription_yaml_depth_budget"));
                }
            }
            line_start = false;
        }
        if byte == b'\n' {
            comment = false;
            line_start = true;
        }
        if comment {
            i += 1;
            continue;
        }
        if quote == b'\'' {
            if byte == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                quote = 0;
            }
        } else if quote == b'"' {
            if byte == b'\\' {
                i += 2;
                continue;
            }
            if byte == b'"' {
                quote = 0;
            }
        } else {
            let start = i == 0 || boundary(bytes[i - 1]);
            match byte {
                b'#' if start => comment = true,
                b'\'' | b'"' if start => quote = byte,
                b'&' | b'*' if start => {
                    return Err(FetchError::known(
                        "subscription_yaml_references_unsupported",
                    ));
                }
                b'[' | b'{' => {
                    flow_depth += 1;
                    if flow_depth > 32 {
                        return Err(FetchError::known("subscription_yaml_depth_budget"));
                    }
                }
                b']' | b'}' => flow_depth = flow_depth.saturating_sub(1),
                _ => {}
            }
        }
        i += 1;
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClashProxy {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    server: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    #[serde(default)]
    tls: bool,
    sni: Option<String>,
    #[serde(default, rename = "skip-cert-verify")]
    skip_cert_verify: bool,
    udp: Option<bool>,
    #[serde(default)]
    tfo: bool,
    #[serde(default)]
    mptcp: bool,
    #[serde(rename = "ip-version")]
    ip_version: Option<String>,
}
fn parse_clash(document: &str) -> Result<Vec<UpstreamConfig>, FetchError> {
    yaml_budget(document)?;
    let doc: ClashDocument = serde_yaml_ng::from_str(document)
        .map_err(|_| FetchError::known("invalid_or_unsupported_clash_subscription"))?;
    if doc.proxies.len() > 10000 {
        return Err(FetchError::known("subscription_node_budget"));
    }
    doc.proxies
        .into_iter()
        .map(|p| {
            if p.name.is_empty()
                || p.name.len() > 256
                || p.skip_cert_verify
                || p.tfo
                || p.mptcp
                || p.ip_version.is_some()
                || (p.kind == "socks5" && (p.tls || p.sni.is_some() || p.udp == Some(false)))
                || (p.kind == "http" && p.udp == Some(true))
                || (!p.tls && p.sni.is_some())
            {
                return Err(FetchError::known("unsupported_clash_proxy_option"));
            }
            let protocol = match p.kind.as_str() {
                "http" if p.tls => ProxyProtocol::Https,
                "http" => ProxyProtocol::Http,
                "socks5" => ProxyProtocol::Socks5,
                _ => return Err(FetchError::known("unsupported_subscription_protocol")),
            };
            let node = UpstreamConfig {
                id: "preview".into(),
                protocol,
                host: p.server,
                port: p.port,
                enabled: true,
                credentials: if p.username.is_some() || p.password.is_some() {
                    Some(UpstreamCredentials {
                        username: p.username.unwrap_or_default(),
                        password: p.password.unwrap_or_default(),
                    })
                } else {
                    None
                },
                expires_at_unix: None,
                ca_certificate: None,
                tls_server_name: p.sni,
            };
            hps_config::ProxyConfig {
                upstreams: vec![node.clone()],
                ..Default::default()
            }
            .validate()
            .map_err(|_| FetchError::known("invalid_subscription_endpoint"))?;
            Ok(node)
        })
        .collect()
}
pub async fn fetch(
    config: &SubscriptionConfig,
    resolver: Option<Arc<dyn hps_net::NameResolver>>,
) -> Result<Vec<UpstreamConfig>, FetchError> {
    config
        .validate()
        .map_err(|_| FetchError::known("invalid_subscription"))?;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_millis(config.timeout_ms))
        .connect_timeout(Duration::from_millis(config.timeout_ms.min(10000)))
        .user_agent("honk-proxy-server/subscription");
    if let Some(resolver) = resolver {
        builder = builder.dns_resolver(Arc::new(ProviderResolver(resolver)));
    }
    let client = builder
        .build()
        .map_err(|_| FetchError::known("subscription_client_failed"))?;
    let mut response = client
        .get(&config.url)
        .send()
        .await
        .map_err(|_| FetchError::known("subscription_fetch_failed"))?;
    if !response.status().is_success() {
        return Err(FetchError::known("subscription_http_status"));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err(FetchError::known("subscription_body_budget"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| FetchError::known("subscription_body_failed"))?
    {
        if chunk.len() > MAX_BYTES - bytes.len() {
            return Err(FetchError::known("subscription_body_budget"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let text =
        String::from_utf8(bytes).map_err(|_| FetchError::known("subscription_invalid_utf8"))?;
    let config = config.clone();
    // The caller's tracked operation and provider slot remain owned until this
    // blocking parse finishes, including if the HTTP caller disappears.
    tokio::task::spawn_blocking(move || parse(&config, &text))
        .await
        .map_err(|_| FetchError::known("subscription_parse_worker_failed"))?
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config(format: SubscriptionFormat) -> SubscriptionConfig {
        SubscriptionConfig {
            id: "office".into(),
            url: "https://example.test/sub?token=private-token".into(),
            format,
            ..Default::default()
        }
    }
    #[test]
    fn uri_and_base64_preserve_credentials_dual_stack_and_stable_ids_without_fabricated_expiry() {
        let c = config(SubscriptionFormat::Uri);
        let raw =
            "socks5://user:p%40ss@[::1]:1080#%E8%8A%82%E7%82%B9\nhttps://example.test:443#secure";
        let nodes = parse(&c, raw).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].credentials.as_ref().unwrap().password, "p@ss");
        assert_eq!(nodes[0].host, "::1");
        assert!(nodes.iter().all(|n| n.expires_at_unix.is_none()));
        let encoded = STANDARD.encode(raw);
        assert_eq!(nodes[0].id, parse(&c, &encoded).unwrap()[0].id);
        assert!(!format!("{c:?}").contains("private-token"));
        assert!(parse(&c, "ss://unsupported").is_err());
        assert!(parse(&c, "").is_err());
        assert!(parse(&c, "http://a:80\nhttp://a:80").is_err());
    }
    #[test]
    fn clash_supported_subset_is_strict_and_sensitive_parse_errors_are_constant() {
        let c = config(SubscriptionFormat::Clash);
        let raw = "proxies:\n  - {name: 香港, type: socks5, server: '::1', port: 1080, username: user, password: secret, udp: true}\n  - {name: web, type: http, server: proxy.example, port: 443, tls: true, sni: proxy.example}\nrules: [MATCH,DIRECT]";
        let nodes = parse(&c, raw).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[1].protocol, ProxyProtocol::Https);
        for bad in [
            raw.replace("udp: true", "udp: false"),
            raw.replace("type: socks5", "type: vmess"),
            raw.replace("tls: true", "tls: true, skip-cert-verify: true"),
            raw.replace("password: secret", "password: [secret]"),
        ] {
            let e = parse(&c, &bad).unwrap_err().to_string();
            assert!(!e.contains("secret"));
        }
    }
    #[test]
    fn clash_alias_depth_and_streamed_entry_budgets_apply_before_expansion() {
        let mut c = config(SubscriptionFormat::Clash);
        c.max_nodes = 10000;
        for text in [
            "proxies: &nodes []",
            "anchor: &node {name: a}\nproxies: [*node]",
        ] {
            assert_eq!(
                parse(&c, text).unwrap_err().code,
                "subscription_yaml_references_unsupported"
            );
        }
        let quoted = "proxies:\n - {name: \"*literal &name\", type: socks5, server: 127.0.0.1, port: 1080, username: user, password: '&literal *password', udp: true} # *comment &comment";
        assert_eq!(
            parse(&c, quoted).unwrap()[0]
                .credentials
                .as_ref()
                .unwrap()
                .password,
            "&literal *password"
        );
        let flow = format!("proxies: {}0{}", "[".repeat(33), "]".repeat(33));
        assert_eq!(
            parse(&c, &flow).unwrap_err().code,
            "subscription_yaml_depth_budget"
        );
        let block = (0..33)
            .map(|n| format!("{}level:\n", " ".repeat(n)))
            .collect::<String>();
        assert_eq!(
            parse(&c, &block).unwrap_err().code,
            "subscription_yaml_depth_budget"
        );
        let document = format!(
            "proxies:\n{}",
            " - {name: n, type: http, server: 127.0.0.1, port: 8080}\n".repeat(10001)
        );
        assert!(document.len() < MAX_BYTES);
        assert_eq!(
            parse(&c, &document).unwrap_err().code,
            "invalid_or_unsupported_clash_subscription"
        );
    }
}
