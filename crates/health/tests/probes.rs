#[path = "../../engine/tests/support/mod.rs"]
mod support;
use hps_config::{HealthConfig, ProbeProfile, ProxyConfig, ProxyProtocol, UpstreamCredentials};
use hps_domain::{HealthTransport, HealthVerdict, IpFamily, ProbePhase};
use hps_engine::dial::Dialer;
use hps_health::PreparedProfile;
use std::sync::Arc;

#[tokio::test]
async fn real_three_protocols_probe_verified_target_tls_without_business_payload() {
    let cert = support::certificate();
    let target = support::origin(Some(&cert)).await;
    let profile = PreparedProfile::prepare(ProbeProfile {
        url: format!("https://127.0.0.1:{}/health", target.address.port()),
        ca_certificate: Some(cert.config.certificate.clone()),
        tls_server_name: Some("localhost".into()),
        ..Default::default()
    })
    .unwrap();
    for protocol in [
        ProxyProtocol::Http,
        ProxyProtocol::Https,
        ProxyProtocol::Socks5,
    ] {
        let proxy = if protocol == ProxyProtocol::Socks5 {
            support::socks_proxy(true).await
        } else {
            support::http_proxy(
                (protocol == ProxyProtocol::Https).then_some(&cert),
                Some("Basic c3VwcGxpZXI6c2VjcmV0"),
                200,
            )
            .await
        };
        let mut node = support::upstream(proxy.address, protocol);
        node.credentials = Some(UpstreamCredentials {
            username: "supplier".into(),
            password: "secret".into(),
        });
        if protocol == ProxyProtocol::Https {
            node.ca_certificate = Some(cert.config.certificate.clone());
            node.tls_server_name = Some("localhost".into());
        }
        let dialer = Arc::new(
            Dialer::new(&ProxyConfig {
                upstreams: vec![node],
                ..Default::default()
            })
            .unwrap(),
        );
        let result = profile
            .probe(&dialer.probe_plans()[0], &HealthConfig::default())
            .await;
        assert_eq!(result.verdict, HealthVerdict::Healthy, "{result:?}");
        assert!(result.endpoint_reachable && result.tunnel_established);
        assert_eq!(result.authenticated, Some(true));
        assert_eq!(result.http_status, Some(200));
    }
    let requests = target.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    for request in requests.iter() {
        let request = String::from_utf8_lossy(request);
        assert!(request.contains("/health"));
        assert!(!request.contains("cookie") && !request.contains("proxy-authorization"));
    }
}

#[tokio::test]
async fn auth_endpoint_upstream_tls_and_target_tls_are_separate() {
    let cert = support::certificate();
    let target = support::origin(Some(&cert)).await;
    let profile = PreparedProfile::prepare(ProbeProfile {
        url: format!("https://127.0.0.1:{}/", target.address.port()),
        ..Default::default()
    })
    .unwrap();
    for (status, expected, phase) in [
        (407, HealthVerdict::Failed, ProbePhase::Authentication),
        (403, HealthVerdict::Degraded, ProbePhase::ProxyTunnel),
    ] {
        let proxy = support::http_proxy(None, None, status).await;
        let dialer = Arc::new(
            Dialer::new(&ProxyConfig {
                upstreams: vec![support::upstream(proxy.address, ProxyProtocol::Http)],
                ..Default::default()
            })
            .unwrap(),
        );
        let result = profile
            .probe(&dialer.probe_plans()[0], &HealthConfig::default())
            .await;
        assert_eq!(result.verdict, expected, "{result:?}");
        assert_eq!(result.phase, phase);
        assert_eq!(proxy.requests.lock().unwrap().len(), 1);
    }
    let tls_proxy = support::http_proxy(Some(&cert), None, 200).await;
    let dialer = Arc::new(
        Dialer::new(&ProxyConfig {
            upstreams: vec![support::upstream(tls_proxy.address, ProxyProtocol::Https)],
            ..Default::default()
        })
        .unwrap(),
    );
    let result = profile
        .probe(&dialer.probe_plans()[0], &HealthConfig::default())
        .await;
    assert_eq!(result.phase, ProbePhase::UpstreamTls);
    assert_eq!(result.verdict, HealthVerdict::Failed);
    let proxy = support::http_proxy(None, None, 200).await;
    let dialer = Arc::new(
        Dialer::new(&ProxyConfig {
            upstreams: vec![support::upstream(proxy.address, ProxyProtocol::Http)],
            ..Default::default()
        })
        .unwrap(),
    );
    let result = profile
        .probe(&dialer.probe_plans()[0], &HealthConfig::default())
        .await;
    assert_eq!(result.phase, ProbePhase::TargetTls);
    assert_eq!(result.verdict, HealthVerdict::Degraded);
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);
    let dialer = Arc::new(
        Dialer::new(&ProxyConfig {
            upstreams: vec![support::upstream(address, ProxyProtocol::Http)],
            ..Default::default()
        })
        .unwrap(),
    );
    let result = profile
        .probe(&dialer.probe_plans()[0], &HealthConfig::default())
        .await;
    assert_eq!(result.phase, ProbePhase::EndpointConnect);
    assert!(!result.endpoint_reachable);
}

#[tokio::test]
async fn target_403_and_429_are_reachable_not_globally_dead() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for status in [403, 429] {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut io: hps_engine::BoxIo = Box::new(tcp);
            let _ = support::head(&mut io).await;
            io.write_all(
                format!("HTTP/1.1 {status} Rejected\r\nContent-Length: 0\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
        }
    });
    let proxy = support::http_proxy(None, None, 200).await;
    let dialer = Arc::new(
        Dialer::new(&ProxyConfig {
            upstreams: vec![support::upstream(proxy.address, ProxyProtocol::Http)],
            ..Default::default()
        })
        .unwrap(),
    );
    let profile = PreparedProfile::prepare(ProbeProfile {
        url: format!("http://{address}/"),
        ..Default::default()
    })
    .unwrap();
    for status in [403, 429] {
        let result = profile
            .probe(&dialer.probe_plans()[0], &HealthConfig::default())
            .await;
        assert_eq!(result.verdict, HealthVerdict::Healthy);
        assert_eq!(result.target_ok, Some(false));
        assert_eq!(result.http_status, Some(status));
        assert_eq!(result.error_code.as_deref(), Some("target_rejected"));
    }
    server.await.unwrap();
}

#[tokio::test]
async fn short_lifetime_and_udp_do_not_touch_supplier_and_family_is_explicit() {
    let proxy = support::http_proxy(None, None, 200).await;
    let mut node = support::upstream(proxy.address, ProxyProtocol::Http);
    node.expires_at_unix = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 20,
    );
    let dialer = Arc::new(
        Dialer::new(&ProxyConfig {
            upstreams: vec![node],
            ..Default::default()
        })
        .unwrap(),
    );
    let profile = PreparedProfile::prepare(ProbeProfile::default()).unwrap();
    let result = profile
        .probe(&dialer.probe_plans()[0], &HealthConfig::default())
        .await;
    assert_eq!(result.verdict, HealthVerdict::Skipped);
    let mut udp = ProbeProfile::default();
    udp.transport = HealthTransport::Udp;
    udp.enabled = false;
    let result = PreparedProfile::prepare(udp)
        .unwrap()
        .probe(&dialer.probe_plans()[0], &HealthConfig::default())
        .await;
    assert_eq!(result.verdict, HealthVerdict::Unsupported);
    assert!(proxy.requests.lock().unwrap().is_empty());
    assert!(
        PreparedProfile::prepare(ProbeProfile {
            url: "http://[::1]/".into(),
            family: IpFamily::Ipv4,
            ..Default::default()
        })
        .is_err()
    );
}

#[tokio::test]
async fn ipv6_is_actually_dialed_and_timeout_preserves_phase() {
    use tokio::io::AsyncWriteExt;
    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut io: hps_engine::BoxIo = Box::new(tcp);
        let _ = support::head(&mut io).await;
        io.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let proxy = support::socks_proxy(false).await;
    let dialer = Arc::new(
        Dialer::new(&ProxyConfig {
            upstreams: vec![support::upstream(proxy.address, ProxyProtocol::Socks5)],
            ..Default::default()
        })
        .unwrap(),
    );
    let profile = PreparedProfile::prepare(ProbeProfile {
        url: format!("http://{address}/"),
        family: IpFamily::Ipv6,
        ..Default::default()
    })
    .unwrap();
    let result = profile
        .probe(&dialer.probe_plans()[0], &HealthConfig::default())
        .await;
    assert_eq!(result.verdict, HealthVerdict::Healthy, "{result:?}");
    assert_eq!(result.family, IpFamily::Ipv6);
    server.await.unwrap();
    assert_eq!(&*proxy.requests.lock().unwrap(), &[b"::1".to_vec()]);
    let blackhole = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let node = support::upstream(blackhole.local_addr().unwrap(), ProxyProtocol::Http);
    let dialer = Arc::new(
        Dialer::new(&ProxyConfig {
            upstreams: vec![node],
            ..Default::default()
        })
        .unwrap(),
    );
    let profile = PreparedProfile::prepare(ProbeProfile {
        url: "https://127.0.0.1/".into(),
        ..Default::default()
    })
    .unwrap();
    let result = profile
        .probe(
            &dialer.probe_plans()[0],
            &HealthConfig {
                timeout_ms: 100,
                ..Default::default()
            },
        )
        .await;
    assert_eq!(result.phase, ProbePhase::ProxyTunnel);
    assert_eq!(
        result.error_code.as_deref(),
        Some("proxy_handshake_timeout")
    );
}
