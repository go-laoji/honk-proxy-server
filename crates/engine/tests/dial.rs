mod support;
use hps_config::{ProxyConfig, ProxyProtocol, UpstreamCredentials};
use hps_engine::{ProxyError, dial::Dialer, target::Target};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn real_http_https_and_authenticated_socks_tunnels_preserve_half_close() {
    let destination = support::echo(false).await;
    let cert = support::certificate();
    for protocol in [
        ProxyProtocol::Http,
        ProxyProtocol::Https,
        ProxyProtocol::Socks5,
    ] {
        let server = if protocol == ProxyProtocol::Socks5 {
            support::socks_proxy(true).await
        } else {
            support::http_proxy(
                (protocol == ProxyProtocol::Https).then_some(&cert),
                Some("Basic c3VwcGxpZXI6c2VjcmV0"),
                200,
            )
            .await
        };
        let mut upstream = support::upstream(server.address, protocol);
        upstream.credentials = Some(UpstreamCredentials {
            username: "supplier".into(),
            password: "secret".into(),
        });
        if protocol == ProxyProtocol::Https {
            upstream.ca_certificate = Some(cert.config.certificate.clone());
            upstream.tls_server_name = Some("localhost".into());
        }
        let dialer = Dialer::new(&ProxyConfig {
            upstreams: vec![upstream],
            ..Default::default()
        })
        .unwrap();
        let mut tunnel = dialer
            .connect(
                &Target::new("localhost", destination.address.port()).unwrap(),
                false,
            )
            .await
            .unwrap();
        assert!(!tunnel.absolute_form);
        assert!(tunnel.proxy_authorization.is_none());
        let data = vec![19; 1024 * 1024];
        tunnel.io.write_all(&data).await.unwrap();
        tunnel.io.shutdown().await.unwrap();
        let mut echoed = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tunnel.io.read_to_end(&mut echoed),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(data, echoed);
    }
}

#[tokio::test]
async fn untrusted_or_wrong_name_tls_and_upstream_407_fail_without_retry() {
    let cert = support::certificate();
    let server = support::http_proxy(Some(&cert), None, 200).await;
    let mut upstream = support::upstream(server.address, ProxyProtocol::Https);
    let target = Target::new("example.test", 443).unwrap();
    let config = |upstream| ProxyConfig {
        upstreams: vec![upstream],
        ..Default::default()
    };
    assert!(
        Dialer::new(&config(upstream.clone()))
            .unwrap()
            .connect(&target, false)
            .await
            .is_err()
    );
    upstream.ca_certificate = Some(cert.config.certificate.clone());
    upstream.tls_server_name = Some("wrong.test".into());
    assert!(
        Dialer::new(&config(upstream))
            .unwrap()
            .connect(&target, false)
            .await
            .is_err()
    );
    let refused = support::http_proxy(None, None, 407).await;
    let unused = support::http_proxy(None, None, 200).await;
    let mut second = support::upstream(unused.address, ProxyProtocol::Http);
    second.id = "second".into();
    let dialer = Dialer::new(&ProxyConfig {
        upstreams: vec![
            support::upstream(refused.address, ProxyProtocol::Http),
            second,
        ],
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(
        dialer.connect(&target, false).await,
        Err(ProxyError::Authentication)
    ));
    assert_eq!(refused.requests.lock().unwrap().len(), 1);
    assert!(unused.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn expired_near_expiry_and_disabled_nodes_are_not_contacted() {
    let server = support::http_proxy(None, None, 200).await;
    for (expires, enabled) in [
        (Some(1), true),
        (
            Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    + 10,
            ),
            true,
        ),
        (None, false),
    ] {
        let mut upstream = support::upstream(server.address, ProxyProtocol::Http);
        upstream.expires_at_unix = expires;
        upstream.enabled = enabled;
        let dialer = Dialer::new(&ProxyConfig {
            upstreams: vec![upstream],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(dialer.eligible_count(), 0);
        assert!(matches!(
            dialer
                .connect(&Target::new("127.0.0.1", 80).unwrap(), false)
                .await,
            Err(ProxyError::NoUpstream)
        ));
    }
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn socks_ipv6_and_http_forward_plan_are_supported() {
    let destination = support::echo(true).await;
    let socks = support::socks_proxy(false).await;
    let dialer = Dialer::new(&ProxyConfig {
        upstreams: vec![support::upstream(socks.address, ProxyProtocol::Socks5)],
        ..Default::default()
    })
    .unwrap();
    let mut tunnel = dialer
        .connect(
            &Target::new("::1", destination.address.port()).unwrap(),
            false,
        )
        .await
        .unwrap();
    tunnel.io.write_all(b"ipv6").await.unwrap();
    tunnel.io.shutdown().await.unwrap();
    let mut bytes = Vec::new();
    tunnel.io.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(bytes, b"ipv6");
    let http = support::http_proxy(None, None, 200).await;
    let dialer = Dialer::new(&ProxyConfig {
        upstreams: vec![support::upstream(http.address, ProxyProtocol::Http)],
        ..Default::default()
    })
    .unwrap();
    let connection = dialer
        .connect(&Target::new("origin.test", 80).unwrap(), true)
        .await
        .unwrap();
    assert!(connection.absolute_form);
}
