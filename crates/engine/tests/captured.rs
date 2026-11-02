#![cfg(target_os = "linux")]
mod support;
use hps_config::{
    GroupConfig, ProcessCondition, ProxyConfig, ProxyProtocol, RuleConditions, RuleConfig,
};
use hps_domain::{FlowOrigin, ProcessIdentity, RouteAction, RoutingContext, SelectionTransport};
use hps_engine::{ProxyError, runtime::Engine};
use std::{net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn context(source: SocketAddr, destination: SocketAddr) -> RoutingContext {
    RoutingContext {
        destination_host: destination.ip().to_string(),
        destination_port: destination.port(),
        transport: SelectionTransport::Tcp,
        source_ip: Some(source.ip()),
        source_port: Some(source.port()),
        inbound: None,
        inbound_protocol: Some("transparent".into()),
        observed_domains: vec![],
        origin: FlowOrigin::LinuxLocal,
        process: Some(ProcessIdentity {
            pid: 1234,
            command: "hps-client".into(),
            cgroup_id: 77,
        }),
    }
}

#[tokio::test]
async fn captured_tcp_contract_shares_process_rules_group_admission_bytes_disconnect_and_immutable_revision()
 {
    for protocol in [
        ProxyProtocol::Http,
        ProxyProtocol::Https,
        ProxyProtocol::Socks5,
    ] {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = target.local_addr().unwrap();
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
        node.id = "selected".into();
        if protocol == ProxyProtocol::Https {
            node.ca_certificate = Some(cert.config.certificate.clone());
            node.tls_server_name = Some("localhost".into());
        }
        let mut config = ProxyConfig {
            inbounds: vec![],
            upstreams: vec![node],
            ..Default::default()
        };
        config.limits.max_connections = 1;
        config.selection.groups = vec![GroupConfig {
            id: "only-selected".into(),
            members: vec!["selected".into()],
            ..Default::default()
        }];
        config.routing.default_action = RouteAction::Reject;
        config.routing.rules = vec![RuleConfig {
            id: "local-process".into(),
            enabled: true,
            conditions: RuleConditions {
                inbound_protocols: vec!["transparent".into()],
                process: Some(ProcessCondition {
                    command: Some("hps-client".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            action: RouteAction::Group {
                id: "only-selected".into(),
            },
        }];
        assert!(Engine::new(config.clone()).is_err());
        let engine = Engine::with_capture(
            config,
            None,
            hps_engine::routing::RoutingCapabilities {
                local_process: true,
                udp: false,
            },
        )
        .unwrap();
        engine.set_generation(7);
        engine.set_generation(8);
        assert_eq!(engine.generation(), 7);
        let registry = engine.node_registry();
        let budget = hps_engine::capture::CaptureBudget::new(&hps_config::LinuxCaptureConfig {
            object: "/fixture/not-loaded".into(),
            local_cgroup: Some("/fixture/not-attached".into()),
            max_pending: 1,
            ..Default::default()
        })
        .unwrap();
        let scope = tokio_util::sync::CancellationToken::new();

        let mut stale = TcpStream::connect(destination).await.unwrap();
        let (stream, source) = target.accept().await.unwrap();
        assert!(matches!(
            engine.spawn_captured_tcp(stream, context(source, destination), 8),
            Err(ProxyError::UnsupportedRule)
        ));
        assert_eq!(
            stale.read_u8().await.unwrap_err().kind(),
            std::io::ErrorKind::UnexpectedEof
        );
        assert_eq!(registry.connection_records(false).0.started, 0);
        let wrong_source = TcpStream::connect(destination).await.unwrap();
        let (stream, source) = target.accept().await.unwrap();
        let mut invalid = context(source, destination);
        invalid.source_port = Some(source.port().wrapping_add(1));
        assert!(matches!(
            engine.spawn_captured_tcp(stream, invalid, 7),
            Err(ProxyError::InvalidTarget)
        ));
        drop(wrong_source);
        let missing_process = TcpStream::connect(destination).await.unwrap();
        let (stream, source) = target.accept().await.unwrap();
        let mut invalid = context(source, destination);
        invalid.process = None;
        assert!(matches!(
            engine.spawn_captured_tcp(stream, invalid, 7),
            Err(ProxyError::UnsupportedRule)
        ));
        drop(missing_process);

        let mut client = TcpStream::connect(destination).await.unwrap();
        let (stream, source) = target.accept().await.unwrap();
        let captured = engine
            .spawn_captured_tcp_scoped(
                stream,
                context(source, destination),
                7,
                budget.tcp_admission(&scope).unwrap(),
            )
            .unwrap();
        client.write_all(b"opaque-business-marker").await.unwrap();
        let (mut origin, _) = tokio::time::timeout(Duration::from_secs(3), target.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = [0; 22];
        origin.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"opaque-business-marker");
        origin.write_all(b"selected-upstream-reply").await.unwrap();
        let mut reply = [0; 23];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"selected-upstream-reply");
        let (stats, flows) = registry.connection_records(false);
        assert_eq!(stats.active, 1);
        let flow = &flows[0];
        assert_eq!(flow.routing.configuration_revision, 7);
        assert_eq!(flow.routing.rule_id.as_deref(), Some("local-process"));
        assert_eq!(flow.context.process.as_ref().unwrap().pid, 1234);
        assert_eq!(flow.context.source_port, Some(source.port()));
        assert_eq!(
            flow.selection.as_ref().unwrap().group.as_deref(),
            Some("only-selected")
        );
        assert_eq!(flow.choice.as_ref().unwrap().node_id, "selected");
        assert_eq!(flow.bytes_up, 22);
        assert_eq!(flow.bytes_down, 23);
        assert_eq!(budget.usage().pending, 0);
        assert_eq!(registry.active("selected"), 1);
        assert!(!captured.is_finished());
        assert!(!engine.is_idle());

        let capacity_client = TcpStream::connect(destination).await.unwrap();
        let (stream, source) = target.accept().await.unwrap();
        assert!(matches!(
            engine.spawn_captured_tcp(stream, context(source, destination), 7),
            Err(ProxyError::LocalResource)
        ));
        drop(capacity_client);
        assert!(registry.disconnect_connection(&flow.id));
        assert!(
            tokio::time::timeout(Duration::from_secs(2), client.read_u8())
                .await
                .unwrap()
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(2), captured.finished())
            .await
            .unwrap();
        assert!(captured.is_finished());
        assert!(engine.is_idle());
        assert_eq!(budget.usage().pending, 0);
        assert_eq!(registry.active("selected"), 0);
        assert_eq!(proxy.requests.lock().unwrap().len(), 1);

        let mut gateway = TcpStream::connect(destination).await.unwrap();
        let (stream, source) = target.accept().await.unwrap();
        let mut gateway_context = context(source, destination);
        gateway_context.origin = FlowOrigin::Gateway;
        engine
            .spawn_captured_tcp(stream, gateway_context, 7)
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), gateway.read_u8())
                .await
                .unwrap()
                .is_err()
        );
        engine.shutdown().await;
        assert!(engine.is_idle());
        let (_, flows) = registry.connection_records(true);
        let rejected = flows
            .iter()
            .find(|f| f.context.origin == FlowOrigin::Gateway)
            .unwrap();
        assert!(rejected.context.process.is_none());
        assert_eq!(rejected.state, "rejected");
        assert_eq!(proxy.requests.lock().unwrap().len(), 1);
    }
}
