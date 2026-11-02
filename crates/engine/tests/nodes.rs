mod support;
use hps_config::{ProxyConfig, ProxyProtocol};
use hps_domain::{NodeControl, NodeOperationState, SelectionQuery};
use hps_engine::runtime::Engine;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

#[tokio::test]
async fn actual_tunnel_drains_after_disable_and_generations_share_gate_and_live_count() {
    for protocol in [
        ProxyProtocol::Http,
        ProxyProtocol::Https,
        ProxyProtocol::Socks5,
    ] {
        let destination = support::echo(false).await;
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
        let mut upstream = support::upstream(proxy.address, protocol);
        upstream.id = "node".into();
        if protocol == ProxyProtocol::Https {
            upstream.ca_certificate = Some(cert.config.certificate.clone());
            upstream.tls_server_name = Some("localhost".into());
        }
        let config = ProxyConfig {
            upstreams: vec![upstream],
            inbounds: vec![support::inbound("http", ProxyProtocol::Http, None)],
            ..Default::default()
        };
        let engine = Engine::new(config.clone()).unwrap();
        engine.start_enabled().await.unwrap();
        let address = engine.status().inbounds[0].listen.clone().unwrap();
        let mut connection = TcpStream::connect(&address).await.unwrap();
        connection
            .write_all(
                format!(
                    "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
                    destination.address, destination.address
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = vec![];
        while !response.ends_with(b"\r\n\r\n") {
            response.push(connection.read_u8().await.unwrap());
        }
        assert!(response.starts_with(b"HTTP/1.1 200"));
        let registry = engine.node_registry();
        assert_eq!(registry.active("node"), 1);
        registry.publish(NodeControl {
            node_id: "node".into(),
            state: NodeOperationState::Draining,
            version: 1,
            updated_at: 1,
        });
        let replacement = Engine::with_previous(config, Some(&engine)).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &registry,
            &replacement.node_registry()
        ));
        assert_eq!(replacement.node_registry().active("node"), 1);
        assert!(
            replacement.selection(&SelectionQuery::default()).candidates[0]
                .reasons
                .contains(&"draining".into())
        );
        let mut blocked = TcpStream::connect(&address).await.unwrap();
        blocked
            .write_all(
                format!(
                    "CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n",
                    destination.address, destination.address
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut denied = [0; 12];
        blocked.read_exact(&mut denied).await.unwrap();
        assert!(denied.starts_with(b"HTTP/1.1 503"));
        assert_eq!(proxy.requests.lock().unwrap().len(), 1);
        connection
            .write_all(b"still drains existing bytes")
            .await
            .unwrap();
        connection.shutdown().await.unwrap();
        let mut echo = vec![];
        connection.read_to_end(&mut echo).await.unwrap();
        assert_eq!(echo, b"still drains existing bytes");
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while registry.active("node") != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        registry.publish(NodeControl {
            node_id: "node".into(),
            state: NodeOperationState::Enabled,
            version: 2,
            updated_at: 2,
        });
        assert_eq!(
            replacement
                .selection(&SelectionQuery::default())
                .selected
                .as_deref(),
            Some("node")
        );
        engine.shutdown().await;
        replacement.shutdown().await;
    }
}
