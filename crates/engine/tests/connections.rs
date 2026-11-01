mod support;
use hps_config::{ProxyConfig, ProxyProtocol};
use hps_domain::RouteAction;
use hps_engine::runtime::Engine;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[tokio::test]
async fn actual_tunnels_record_real_source_port_policy_bytes_and_disconnect_without_replaying() {
    for protocol in [
        ProxyProtocol::Http,
        ProxyProtocol::Https,
        ProxyProtocol::Socks5,
    ] {
        let target = support::echo(false).await;
        let cert = support::certificate();
        let proxy = if protocol == ProxyProtocol::Socks5 {
            support::socks_proxy(false).await
        } else {
            support::http_proxy(
                (protocol == ProxyProtocol::Https).then_some(&cert),
                None,
                200,
            )
            .await
        };
        let mut node = support::upstream(proxy.address, protocol);
        node.id = "observed".into();
        if protocol == ProxyProtocol::Https {
            node.ca_certificate = Some(cert.config.certificate.clone());
            node.tls_server_name = Some("localhost".into());
        }
        let engine = Engine::new(ProxyConfig {
            upstreams: vec![node],
            inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
            ..Default::default()
        })
        .unwrap();
        engine.set_generation(42);
        engine.start_enabled().await.unwrap();
        let mut client = TcpStream::connect(engine.status().inbounds[0].listen.clone().unwrap())
            .await
            .unwrap();
        let peer = client.local_addr().unwrap();
        client
            .write_all(
                format!(
                    "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
                    target.address, target.address
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = vec![];
        while !response.ends_with(b"\r\n\r\n") {
            response.push(client.read_u8().await.unwrap());
        }
        assert!(response.starts_with(b"HTTP/1.1 200"));
        let registry = engine.node_registry();
        let (stats, flows) = registry.connection_records(false);
        assert_eq!(stats.active, 1);
        let flow = &flows[0];
        assert_eq!(flow.context.source_port, Some(peer.port()));
        assert_eq!(flow.context.source_ip, Some(peer.ip()));
        assert!(flow.context.process.is_none());
        assert_eq!(flow.routing.configuration_revision, 42);
        assert_eq!(flow.choice.as_ref().unwrap().node_id, "observed");
        assert_eq!(
            flow.choice.as_ref().unwrap().upstream_protocol,
            format!("{protocol:?}").to_lowercase()
        );
        assert_eq!(flow.bytes_up, 0);
        assert_eq!(flow.bytes_down, 0); // CONNECT/upstream authentication are outside the business byte boundary.
        client.write_all(b"opaque-business-marker").await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while registry.connection_records(false).0.bytes_up < 22 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            !serde_json::to_string(&registry.connection_records(false).1)
                .unwrap()
                .contains("opaque-business-marker")
        );
        assert!(registry.disconnect_connection(&flow.id));
        let mut bytes = vec![];
        let _ = tokio::time::timeout(Duration::from_secs(1), client.read_to_end(&mut bytes))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while registry.connection_records(false).0.active != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(registry.active("observed"), 0);
        assert_eq!(registry.connection_records(true).1[0].state, "disconnected");
        assert_eq!(proxy.requests.lock().unwrap().len(), 1);
        engine.shutdown().await;
    }
}

#[tokio::test]
async fn direct_uses_same_flow_lifetime_and_counts_while_probe_connections_are_excluded() {
    let target = support::echo(false).await;
    let mut config = ProxyConfig {
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        ..Default::default()
    };
    config.routing.default_action = RouteAction::Direct;
    let engine = Engine::new(config).unwrap();
    engine.start_enabled().await.unwrap();
    let mut client = TcpStream::connect(engine.status().inbounds[0].listen.clone().unwrap())
        .await
        .unwrap();
    client
        .write_all(
            format!(
                "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
                target.address, target.address
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = vec![];
    while !response.ends_with(b"\r\n\r\n") {
        response.push(client.read_u8().await.unwrap());
    }
    let registry = engine.node_registry();
    let (stats, flows) = registry.connection_records(false);
    assert_eq!(stats.active, 1);
    assert_eq!(registry.total_active(), 0);
    assert_eq!(flows[0].routing.action, RouteAction::Direct);
    assert!(flows[0].choice.is_none());
    client.write_all(b"direct-count").await.unwrap();
    client.shutdown().await.unwrap();
    let mut echo = vec![];
    client.read_to_end(&mut echo).await.unwrap();
    assert_eq!(echo, b"direct-count");
    tokio::time::timeout(Duration::from_secs(1), async {
        while registry.connection_records(false).0.active != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let stats = registry.connection_records(false).0;
    assert_eq!(stats.bytes_up, 12);
    assert_eq!(stats.bytes_down, 12);
    engine.shutdown().await;
    let proxy = support::http_proxy(None, None, 200).await;
    let engine = Engine::new(ProxyConfig {
        upstreams: vec![support::upstream(proxy.address, ProxyProtocol::Http)],
        ..Default::default()
    })
    .unwrap();
    let probe = engine.probe_plans()[0]
        .connect(
            &hps_engine::target::Target::new("127.0.0.1", target.address.port()).unwrap(),
            false,
        )
        .await
        .unwrap();
    drop(probe);
    assert_eq!(engine.node_registry().connection_records(true).0.started, 0);
    engine.shutdown().await;
}

#[tokio::test]
async fn real_http_exchange_counts_streamed_bytes_hides_request_content_and_cancels_a_paced_body() {
    let origin = support::origin(None).await;
    let proxy = support::http_proxy(None, None, 200).await;
    let engine = Engine::new(ProxyConfig {
        upstreams: vec![support::upstream(proxy.address, ProxyProtocol::Http)],
        inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
        ..Default::default()
    })
    .unwrap();
    engine.start_enabled().await.unwrap();
    let address = engine.status().inbounds[0].listen.clone().unwrap();
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::http(format!("http://{address}")).unwrap())
        .build()
        .unwrap();
    let response = client
        .post(format!(
            "http://{}/private-path?token=private-query",
            origin.address
        ))
        .header("X-Private-Header", "private-header")
        .body("private-body")
        .send()
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "private-body");
    let registry = engine.node_registry();
    tokio::time::timeout(Duration::from_secs(1), async {
        while registry.connection_records(false).0.active != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (stats, recent) = registry.connection_records(true);
    assert_eq!(stats.started, 1);
    assert!(stats.bytes_up > 12 && stats.bytes_down > 12);
    assert_eq!(recent[0].http_status, Some(200));
    let json = serde_json::to_string(&recent).unwrap();
    for private in [
        "private-path",
        "private-query",
        "private-header",
        "private-body",
    ] {
        assert!(!json.contains(private));
    }
    engine.shutdown().await;
    let mut config = ProxyConfig {
        upstreams: vec![support::upstream(proxy.address, ProxyProtocol::Http)],
        inbounds: vec![support::inbound("paced", ProxyProtocol::Http, None)],
        ..Default::default()
    };
    config.limits.bytes_per_second = 1;
    let engine = Engine::new(config).unwrap();
    engine.start_enabled().await.unwrap();
    let mut socket = TcpStream::connect(engine.status().inbounds[0].listen.clone().unwrap())
        .await
        .unwrap();
    socket
        .write_all(
            format!(
                "POST http://{}/paced HTTP/1.1\r\nHost: {}\r\nContent-Length: 64\r\n\r\n{}",
                origin.address,
                origin.address,
                "x".repeat(64)
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let registry = engine.node_registry();
    let id = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let (_, flows) = registry.connection_records(false);
            if let Some(flow) = flows.first()
                && flow.state == "open"
                && flow.bytes_up > 0
            {
                break flow.id.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(registry.disconnect_connection(&id));
    let mut response = vec![];
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut response))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while registry.connection_records(false).0.active != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(registry.connection_records(false).0.disconnected, 1);
    engine.shutdown().await;
}
