use super::*;
use hps_config::{FetchMethod, JsonFields, ProviderAuth, ProxyProtocol};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn fixture(kind: ProviderKind) -> ProviderConfig {
    ProviderConfig {
        id: "fixture".into(),
        name: "test".into(),
        kind,
        endpoint: "http://127.0.0.1:1/".into(),
        enabled: true,
        ttl_secs: Some(120),
        count: 10,
        ..ProviderConfig::default()
    }
}
#[test]
fn official_hmac_vector_and_rfc3986_encoding() {
    let params = BTreeMap::from([
        ("secret_id".into(), "o1fjh1re9o28876h7c08".into()),
        ("sign_type".into(), "hmacsha1".into()),
        ("timestamp".into(), "1555069980".into()),
    ]);
    assert_eq!(
        sign(
            "GET",
            "/api/getorderexpiretime",
            &params,
            "jd1gzm6ant2u7pojhbtl0bam0xpzsm1c"
        ),
        "ooCUlI6XTxoPS5PG8gNMT37YVl4="
    );
    assert_eq!(encode("成都 a+/="), "%E6%88%90%E9%83%BD%20a%2B%2F%3D");
}
#[test]
fn vendor_envelope_and_no_secret_error_bodies() {
    let config = fixture(ProviderKind::Kuaidaili);
    let result=parse(&config,br#"{"code":0,"data":{"count":2,"order_left_count":7,"proxy_list":["192.0.2.1:8080","[2001:db8::1]:1080"]}}"#,1000).unwrap();
    assert_eq!(result.nodes.len(), 2);
    assert_eq!(result.nodes[0].expires_at, 1120);
    assert_eq!(result.quota, Some(7));
    for (code, expected) in [
        (1, "quota_exhausted"),
        (-6, "whitelist_rejected"),
        (-51, "vendor_source_limit"),
        (-2, "order_unavailable"),
    ] {
        let error = parse(
            &config,
            format!(r#"{{"code":{code},"msg":"private-vendor-token"}}"#).as_bytes(),
            1000,
        )
        .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!format!("{error:?}").contains("private-vendor-token"));
    }
    assert!(
        parse(
            &config,
            br#"{"code":0,"data":{"count":2,"proxy_list":["192.0.2.1:80"]}}"#,
            1000
        )
        .is_err()
    );
}
#[test]
fn generic_json_units_conservative_deadline_and_rejection() {
    let mut config = fixture(ProviderKind::Json);
    config.json = JsonFields {
        expires_at: Some("/expires".into()),
        expiry_milliseconds: true,
        ..JsonFields::default()
    };
    let payload=br#"{"data":{"proxies":[{"ip":"192.0.2.2","port":"8080","ttl":120,"expires":1090000},{"ip":"192.0.2.3","port":80,"ttl":20},{"ip":"192.0.2.4","port":80,"ttl":0}]}}"#;
    let result = parse(&config, payload, 1000).unwrap();
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.nodes[0].expires_at, 1090);
    assert_eq!(result.rejected, 2);
    assert!(
        parse(
            &config,
            br#"{"data":{"proxies":[{"ip":"192.0.2.1","port":80,"ttl":-10}]}}"#,
            1000
        )
        .is_err()
    );
    assert!(
        parse(
            &fixture(ProviderKind::Text),
            b"ERROR(-1): private-token",
            1000
        )
        .is_err()
    );
}
#[test]
fn yun_free_gbk_http_connect_and_local_expiry() {
    let config = fixture(ProviderKind::YunFree);
    let html = "<div id='list'><table><tbody><tr><td>192.0.2.5</td><td>8080</td><td>高匿</td><td>HTTPS</td><td>中国</td></tr><tr><td>bad</td><td>80</td><td></td><td>HTTP</td></tr></tbody></table></div>";
    let (gbk, _, _) = encoding_rs::GBK.encode(html);
    let result = parse(&config, &gbk, 1000).unwrap();
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.nodes[0].upstream.protocol, ProxyProtocol::Http);
    assert_eq!(result.nodes[0].region.as_deref(), Some("中国"));
    assert!(result.nodes[0].expiry_basis.starts_with("local_cache"));
}
#[test]
fn provider_redaction_roundtrip_missing_references_and_fingerprint() {
    let mut original = fixture(ProviderKind::Kuaidaili);
    original.endpoint = "https://dps.kdlapi.com/api/getdps".into();
    original.auth = ProviderAuth::HmacSha1 {
        secret_id: "private-id".into(),
        secret_key: "private-key".into(),
    };
    original
        .headers
        .insert("authorization".into(), "private-header".into());
    original.fetch_proxy = Some("socks5://user:password@localhost:1080".into());
    let mut redacted = original.redacted();
    redacted.validate().unwrap();
    let text = serde_json::to_string(&redacted).unwrap();
    assert!(!text.contains("private-"));
    assert!(redacted.clone().restore_redacted(None).is_err());
    redacted.restore_redacted(Some(&original)).unwrap();
    assert_eq!(fingerprint(&redacted), fingerprint(&original));
    assert!(!format!("{original:?}").contains("private-"));
}
#[tokio::test]
async fn dropped_response_is_not_retried_and_redirects_are_rejected() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            observed.fetch_add(1, Ordering::Relaxed);
            let mut b = [0; 4096];
            let _ = stream.read(&mut b).await;
        }
    });
    let mut config = fixture(ProviderKind::Text);
    config.endpoint = format!("http://{addr}/extract");
    config.timeout_ms = 200;
    let client = ProviderClient::new(&config).unwrap();
    let error = client.fetch(&config, 1000).await.unwrap_err();
    assert!(error.outcome_unknown);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    server.abort();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    config.endpoint = format!("http://{}/extract", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut b = [0; 4096];
        let _ = stream.read(&mut b).await;
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\n\r\n").await.unwrap();
    });
    let error = ProviderClient::new(&config)
        .unwrap()
        .fetch(&config, 1000)
        .await
        .unwrap_err();
    assert_eq!(error.code, "redirect_rejected");
    server.await.unwrap();
}
#[tokio::test]
async fn custom_post_form_is_encoded_once_and_bounded() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = fixture(ProviderKind::Text);
    config.endpoint = format!("http://{}/extract", listener.local_addr().unwrap());
    config.method = FetchMethod::Post;
    config.params.insert("region".into(), "成都 x".into());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut b = vec![0; 4096];
        let n = stream.read(&mut b).await.unwrap();
        let text = String::from_utf8_lossy(&b[..n]);
        assert!(text.starts_with("POST /extract"));
        assert!(text.contains("region=%E6%88%90%E9%83%BD%20x"));
        let body = b"192.0.2.1:8080\n";
        stream
            .write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
            )
            .await
            .unwrap();
        stream.write_all(body).await.unwrap();
    });
    assert_eq!(
        ProviderClient::new(&config)
            .unwrap()
            .fetch(&config, 1000)
            .await
            .unwrap()
            .nodes
            .len(),
        1
    );
    server.await.unwrap();
}

#[tokio::test]
async fn rate_limit_is_an_explicit_rejection_with_bounded_retry_after() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = fixture(ProviderKind::Text);
    config.endpoint = format!("http://{}/extract", listener.local_addr().unwrap());
    let until = httpdate::fmt_http_date(std::time::SystemTime::now() + Duration::from_secs(3600));
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        let _ = stream.read(&mut bytes).await;
        stream.write_all(format!("HTTP/1.1 429 Too Many Requests\r\nRetry-After: {until}\r\nContent-Length: 0\r\n\r\n").as_bytes()).await.unwrap();
    });
    let error = ProviderClient::new(&config)
        .unwrap()
        .fetch(&config, unix_now())
        .await
        .unwrap_err();
    assert_eq!(error.code, "rate_limited");
    assert!(!error.outcome_unknown);
    assert!(
        error
            .retry_after_secs
            .is_some_and(|n| (3598..=3600).contains(&n))
    );
    server.await.unwrap();
}
