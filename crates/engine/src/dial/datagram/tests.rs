use super::*;
use crate::udp::{decode_datagram, encode_datagram};
use hps_config::{GroupConfig, GroupStrategy};
use hps_domain::{
    FlowOrigin, HealthTransport, HealthVerdict, IpFamily, NodeControl, NodeOperationState,
    ProbeResult,
};
use std::{sync::atomic::AtomicUsize, time::UNIX_EPOCH};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

struct Server {
    address: SocketAddr,
    associations: Arc<AtomicUsize>,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}
async fn socks_udp() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let associations = Arc::new(AtomicUsize::new(0));
    let accepted = associations.clone();
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut control, peer) = tokio::select! {biased;_=stop.cancelled()=>return,result=listener.accept()=>result.unwrap()};
            let stop = stop.clone();
            let accepted = accepted.clone();
            tokio::spawn(async move {
                let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let mut greeting = [0; 3];
                control.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting, [5, 1, 0]);
                control.write_all(&[5, 0]).await.unwrap();
                let mut request = [0; 10];
                control.read_exact(&mut request).await.unwrap();
                assert_eq!(request, [5, 3, 0, 1, 0, 0, 0, 0, 0, 0]);
                let mut reply = vec![5, 0, 0, 1, 127, 0, 0, 1];
                reply.extend_from_slice(&relay.local_addr().unwrap().port().to_be_bytes());
                control.write_all(&reply).await.unwrap();
                accepted.fetch_add(1, Ordering::Relaxed);
                let mut buffer = vec![0; 65536];
                let mut control_byte = [0; 1];
                loop {
                    let (length, client) = tokio::select! {biased;_=stop.cancelled()=>return,_=control.read(&mut control_byte)=>return,result=relay.recv_from(&mut buffer)=>result.unwrap()};
                    assert_eq!(client.ip(), peer.ip());
                    let (target, payload) = decode_datagram(&buffer[..length]).unwrap();
                    let destination = SocketAddr::new(target.host.parse().unwrap(), target.port);
                    let outgoing = UdpSocket::bind(if destination.is_ipv4() {
                        "0.0.0.0:0"
                    } else {
                        "[::]:0"
                    })
                    .await
                    .unwrap();
                    outgoing.connect(destination).await.unwrap();
                    outgoing.send(payload).await.unwrap();
                    let mut response = vec![0; 65536];
                    let length = tokio::select! {biased;_=stop.cancelled()=>return,_=control.read(&mut control_byte)=>return,result=outgoing.recv(&mut response)=>result.unwrap()};
                    let mut wrong = target.clone();
                    wrong.port = wrong.port.wrapping_add(1).max(1);
                    relay
                        .send_to(
                            &encode_datagram(&wrong, b"wrong-reported-source").unwrap(),
                            client,
                        )
                        .await
                        .unwrap();
                    relay
                        .send_to(
                            &encode_datagram(&target, &response[..length]).unwrap(),
                            client,
                        )
                        .await
                        .unwrap();
                }
            });
        }
    });
    Server {
        address,
        associations,
        cancel,
        task,
    }
}
fn node(id: &str, address: SocketAddr, protocol: ProxyProtocol) -> UpstreamConfig {
    UpstreamConfig {
        id: id.into(),
        protocol,
        host: address.ip().to_string(),
        port: address.port(),
        enabled: true,
        credentials: None,
        expires_at_unix: None,
        ca_certificate: None,
        tls_server_name: None,
    }
}
fn context(target: &Target) -> RoutingContext {
    RoutingContext {
        destination_host: target.host.clone(),
        destination_port: target.port,
        transport: SelectionTransport::Udp,
        source_ip: Some("127.0.0.1".parse().unwrap()),
        source_port: Some(45678),
        inbound: None,
        inbound_protocol: Some("socks5".into()),
        observed_domains: vec![],
        origin: FlowOrigin::Explicit,
        process: None,
    }
}
async fn echo(ip: &str) -> (Target, tokio::task::JoinHandle<()>) {
    let socket = UdpSocket::bind(SocketAddr::new(ip.parse().unwrap(), 0))
        .await
        .unwrap();
    let address = socket.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut bytes = [0; 1024];
        loop {
            let (length, peer) = socket.recv_from(&mut bytes).await.unwrap();
            socket.send_to(&bytes[..length], peer).await.unwrap();
        }
    });
    (
        Target::new(&address.ip().to_string(), address.port()).unwrap(),
        task,
    )
}
fn healthy(identity: &str, family: IpFamily) -> ProbeResult {
    ProbeResult {
        node_id: "socks".into(),
        identity: identity.into(),
        profile_fingerprint: "tcp-health".into(),
        transport: HealthTransport::Tcp,
        family,
        verdict: HealthVerdict::Healthy,
        phase: ProbePhase::Complete,
        error_code: None,
        endpoint_reachable: true,
        authenticated: None,
        tunnel_established: true,
        http_status: Some(200),
        target_ok: Some(true),
        latency_ms: 0,
        checked_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    }
}

#[tokio::test]
async fn udp_shares_health_fixed_fallback_drain_real_byte_counts_and_management_disconnect() {
    for (ip, family) in [("127.0.0.1", IpFamily::Ipv4), ("::1", IpFamily::Ipv6)] {
        let proxy = socks_udp().await;
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (target, echo) = echo(ip).await;
        let mut config = ProxyConfig {
            upstreams: vec![
                node("http", http.local_addr().unwrap(), ProxyProtocol::Http),
                node("socks", proxy.address, ProxyProtocol::Socks5),
            ],
            ..Default::default()
        };
        config.selection.groups = vec![GroupConfig {
            id: "fixed".into(),
            strategy: GroupStrategy::Fixed,
            members: vec!["http".into(), "socks".into()],
            pinned_node: Some("http".into()),
            fallback_nodes: vec!["socks".into()],
            ..Default::default()
        }];
        config.routing.default_action = RouteAction::Group { id: "fixed".into() };
        let dialer = Dialer::for_datagrams(&config).unwrap();
        dialer.set_revision(11);
        dialer.configure_health(crate::health::HealthPolicy {
            enabled: true,
            profiles: vec![("tcp-health".into(), HealthTransport::Tcp, family)],
            max_staleness_secs: 60,
            traffic_failure_threshold: 2,
        });
        let context = context(&target);
        assert!(matches!(
            dialer.open_udp_routed(&target, &context, 1024).await,
            Err(ProxyError::NoUpstream)
        ));
        assert_eq!(proxy.associations.load(Ordering::Relaxed), 0);
        let identity = dialer
            .upstreams
            .iter()
            .find(|u| u.config.id == "socks")
            .unwrap()
            .identity
            .clone();
        dialer.health.publish(healthy(&identity, family), 60);
        let connection = dialer
            .open_udp_routed(&target, &context, 1024)
            .await
            .unwrap();
        assert_eq!(dialer.nodes.active("socks"), 1);
        assert!(dialer.demand.active());
        let mut expected_bytes = 0;
        for payload in [
            b"first-business".as_slice(),
            b"existing-during-drain".as_slice(),
        ] {
            if expected_bytes > 0 {
                dialer.nodes.publish(NodeControl {
                    node_id: "socks".into(),
                    state: NodeOperationState::Draining,
                    version: 1,
                    updated_at: 1,
                });
                assert!(matches!(
                    dialer.open_udp_routed(&target, &context, 1024).await,
                    Err(ProxyError::NoUpstream)
                ));
            }
            assert_eq!(connection.send(payload).await.unwrap(), payload.len());
            let mut buffer = [0; 1024];
            assert!(matches!(
                connection.recv(&mut buffer).await,
                Err(ProxyError::Protocol)
            ));
            let (length, source) = connection.recv(&mut buffer).await.unwrap();
            assert_eq!(&buffer[..length], payload);
            assert_eq!(source, target);
            expected_bytes += payload.len() as u64;
        }
        let (stats, flows) = dialer.nodes.connection_records(false);
        assert_eq!(stats.bytes_up, expected_bytes);
        assert_eq!(stats.bytes_down, expected_bytes);
        assert_eq!(stats.active, 1);
        let flow = &flows[0];
        assert_eq!(flow.routing.configuration_revision, 11);
        assert_eq!(
            flow.selection.as_ref().unwrap().group.as_deref(),
            Some("fixed")
        );
        assert_eq!(flow.selection.as_ref().unwrap().reason, "explicit_fallback");
        assert_eq!(flow.choice.as_ref().unwrap().node_id, "socks");
        assert_eq!(flow.context.transport, SelectionTransport::Udp);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), http.accept())
                .await
                .is_err()
        );
        assert!(dialer.nodes.disconnect_connection(&flow.id));
        let mut buffer = [0; 1024];
        assert!(matches!(
            connection.recv(&mut buffer).await,
            Err(ProxyError::Disconnected)
        ));
        assert!(matches!(
            connection.send(b"must-not-replay").await,
            Err(ProxyError::Disconnected)
        ));
        drop(connection);
        assert_eq!(dialer.nodes.active("socks"), 0);
        assert!(!dialer.demand.active());
        assert_eq!(proxy.associations.load(Ordering::Relaxed), 1);
        assert!(
            dialer
                .nodes
                .connection_records(true)
                .1
                .iter()
                .any(|f| f.state == "disconnected")
        );
        echo.abort();
    }
}

#[tokio::test]
async fn direct_and_reject_udp_never_raise_purchase_demand_and_udp_cannot_raise_tcp_provider_demand()
 {
    let (target, echo) = echo("127.0.0.1").await;
    let context = context(&target);
    let mut config = ProxyConfig::default();
    config.routing.default_action = RouteAction::Direct;
    let direct = Dialer::for_datagrams(&config).unwrap();
    let connection = direct
        .open_udp_routed(&target, &context, 1024)
        .await
        .unwrap();
    connection.send(b"direct-datagram").await.unwrap();
    let mut buffer = [0; 64];
    let (length, _) = connection.recv(&mut buffer).await.unwrap();
    assert_eq!(&buffer[..length], b"direct-datagram");
    assert!(!direct.demand.active());
    assert!(direct.demand.age().is_none());
    drop(connection);
    config.routing.default_action = RouteAction::Reject;
    let rejected = Dialer::for_datagrams(&config).unwrap();
    assert!(matches!(
        rejected.open_udp_routed(&target, &context, 1024).await,
        Err(ProxyError::Rejected)
    ));
    assert!(rejected.demand.age().is_none());
    config.routing.default_action = RouteAction::Default;
    let empty = Dialer::for_datagrams(&config).unwrap();
    empty.set_sources(
        [
            ("http-source".into(), "http-fp".into()),
            ("socks-source".into(), "socks-fp".into()),
        ]
        .into(),
    );
    empty.set_source_protocols(
        [
            ("http-source".into(), ProxyProtocol::Http),
            ("socks-source".into(), ProxyProtocol::Socks5),
        ]
        .into(),
    );
    assert!(matches!(
        empty.open_udp_routed(&target, &context, 1024).await,
        Err(ProxyError::NoUpstream)
    ));
    assert!(
        empty
            .demand
            .source(
                "http-source",
                &format!("http-fp:{}:@unbound", empty.refill_fingerprint)
            )
            .1
            .is_none()
    );
    assert!(
        empty
            .demand
            .source(
                "socks-source",
                &format!("socks-fp:{}:@unbound", empty.refill_fingerprint)
            )
            .1
            .is_some()
    );
    echo.abort();
}

#[tokio::test]
async fn expired_udp_node_never_creates_an_upstream_control_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut upstream = node(
        "expired",
        listener.local_addr().unwrap(),
        ProxyProtocol::Socks5,
    );
    upstream.expires_at_unix = Some(0);
    let config = ProxyConfig {
        upstreams: vec![upstream],
        ..Default::default()
    };
    let dialer = Dialer::for_datagrams(&config).unwrap();
    let target = Target::new("203.0.113.88", 53).unwrap();
    assert!(matches!(
        dialer
            .open_udp_routed(&target, &context(&target), 1024)
            .await,
        Err(ProxyError::NoUpstream)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), listener.accept())
            .await
            .is_err()
    );
    let selection = dialer.selection(&hps_domain::SelectionQuery {
        transport: SelectionTransport::Udp,
        ..Default::default()
    });
    assert!(selection.candidates[0].reasons.contains(&"expired".into()));
}
