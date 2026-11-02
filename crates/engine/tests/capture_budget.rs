#![cfg(target_os = "linux")]
use hps_config::{InboundConfig, LinuxCaptureConfig, ProxyConfig, ProxyProtocol, UpstreamConfig};
use hps_domain::{FlowOrigin, ProcessIdentity, RoutingContext, SelectionTransport};
use hps_engine::{
    ProxyError,
    capture::CaptureBudget,
    routing::RoutingCapabilities,
    runtime::Engine,
    udp::{decode_datagram, encode_datagram},
};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Notify, Semaphore},
};
use tokio_util::sync::CancellationToken;

struct Relay {
    address: SocketAddr,
    handshake: Arc<Semaphore>,
    late: Arc<Semaphore>,
    late_seen: Arc<Notify>,
    datagrams: Arc<AtomicUsize>,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
#[derive(Clone, Copy)]
enum Behavior {
    Echo,
    CloseAfterAssociate,
    LargeReplyThenClose,
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}
async fn relay(behavior: Behavior) -> Relay {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handshake = Arc::new(Semaphore::new(0));
    let permit = handshake.clone();
    let late = Arc::new(Semaphore::new(0));
    let allow_late = late.clone();
    let late_seen = Arc::new(Notify::new());
    let seen = late_seen.clone();
    let datagrams = Arc::new(AtomicUsize::new(0));
    let received = datagrams.clone();
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut control, _) = tokio::select! {biased;_=cancel.cancelled()=>return,result=listener.accept()=>result.unwrap()};
            let permit = permit.clone();
            let allow_late = allow_late.clone();
            let seen = seen.clone();
            let received = received.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                let mut bytes = [0; 3];
                control.read_exact(&mut bytes).await.unwrap();
                assert_eq!(bytes, [5, 1, 0]);
                control.write_all(&[5, 0]).await.unwrap();
                let mut command = [0; 10];
                control.read_exact(&mut command).await.unwrap();
                assert_eq!(command, [5, 3, 0, 1, 0, 0, 0, 0, 0, 0]);
                let permission = tokio::select! {biased;_=cancel.cancelled()=>return,result=permit.acquire_owned()=>result.unwrap()};
                permission.forget();
                let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                let mut reply = vec![5, 0, 0, 1, 127, 0, 0, 1];
                reply.extend_from_slice(&socket.local_addr().unwrap().port().to_be_bytes());
                control.write_all(&reply).await.unwrap();
                if matches!(behavior, Behavior::CloseAfterAssociate) {
                    return;
                }
                let mut packet = vec![0; 65536];
                let mut control_byte = [0; 1];
                loop {
                    let (length, peer) = tokio::select! {biased;_=cancel.cancelled()=>return,_=control.read(&mut control_byte)=>return,result=socket.recv_from(&mut packet)=>result.unwrap()};
                    let (target, payload) = decode_datagram(&packet[..length]).unwrap();
                    received.fetch_add(1, Ordering::Relaxed);
                    if payload == b"late-reply" {
                        seen.notify_one();
                        let permission = tokio::select! {biased;_=cancel.cancelled()=>return,result=allow_late.clone().acquire_owned()=>result.unwrap()};
                        permission.forget();
                    }
                    if matches!(behavior, Behavior::LargeReplyThenClose) {
                        socket
                            .send_to(&encode_datagram(&target, &[9; 512]).unwrap(), peer)
                            .await
                            .unwrap();
                        let permission = tokio::select! {biased;_=cancel.cancelled()=>return,result=allow_late.clone().acquire_owned()=>result.unwrap()};
                        permission.forget();
                        return;
                    }
                    socket
                        .send_to(&encode_datagram(&target, payload).unwrap(), peer)
                        .await
                        .unwrap();
                }
            });
        }
    });
    Relay {
        address,
        handshake,
        late,
        late_seen,
        datagrams,
        stop,
        task,
    }
}
fn settings() -> LinuxCaptureConfig {
    LinuxCaptureConfig {
        object: "/fixture/not-loaded".into(),
        local_cgroup: Some("/fixture/not-attached".into()),
        max_pending: 1,
        max_udp_sessions: 1,
        max_datagram_bytes: 512,
        max_queued_datagrams: 2,
        max_queued_bytes: 512,
        udp_idle_secs: 1,
        ..Default::default()
    }
}
fn config(endpoint: SocketAddr) -> ProxyConfig {
    ProxyConfig {
        inbounds: vec![InboundConfig {
            id: "explicit".into(),
            protocol: ProxyProtocol::Http,
            listen: "127.0.0.1:0".parse().unwrap(),
            enabled: true,
            tls: None,
            max_connections: 4,
            bytes_per_second: 0,
        }],
        upstreams: vec![UpstreamConfig {
            id: "socks".into(),
            protocol: ProxyProtocol::Socks5,
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
            enabled: true,
            credentials: None,
            expires_at_unix: None,
            ca_certificate: None,
            tls_server_name: None,
        }],
        ..Default::default()
    }
}
async fn endpoints() -> (UdpSocket, UdpSocket, RoutingContext) {
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let reply = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    reply.connect(client.local_addr().unwrap()).await.unwrap();
    let destination = reply.local_addr().unwrap();
    let source = client.local_addr().unwrap();
    let context = RoutingContext {
        destination_host: destination.ip().to_string(),
        destination_port: destination.port(),
        transport: SelectionTransport::Udp,
        source_ip: Some(source.ip()),
        source_port: Some(source.port()),
        inbound: None,
        inbound_protocol: Some("transparent".into()),
        observed_domains: vec![],
        origin: FlowOrigin::LinuxLocal,
        process: Some(ProcessIdentity {
            pid: 1234,
            command: "fixture".into(),
            cgroup_id: 77,
        }),
    };
    (client, reply, context)
}

#[tokio::test]
async fn udp_owner_bounds_pending_sessions_inflight_bytes_and_cancels_without_stopping_explicit_ingress()
 {
    let relay = relay(Behavior::Echo).await;
    let specification = settings();
    let budget = CaptureBudget::new(&specification).unwrap();
    let scope = CancellationToken::new();
    let mut config = config(relay.address);
    config.limits.max_connections = 1;
    let capabilities = RoutingCapabilities {
        local_process: true,
        udp: true,
    };
    let engine = Engine::with_capture(config.clone(), None, capabilities).unwrap();
    engine.set_generation(7);
    engine.start_enabled().await.unwrap();
    let (client, reply, context) = endpoints().await;
    let session = engine
        .spawn_captured_udp(reply, context, 7, budget.udp_admission(&scope).unwrap())
        .unwrap();
    session.try_send(&[9; 512]).unwrap();
    assert!(matches!(
        session.try_send(&[1]),
        Err(ProxyError::LocalResource)
    ));
    session.try_send(b"").unwrap();
    assert!(matches!(
        session.try_send(b""),
        Err(ProxyError::LocalResource)
    ));
    assert!(matches!(
        session.try_send(&[0; 513]),
        Err(ProxyError::Protocol)
    ));
    let usage = budget.usage();
    assert_eq!(
        (
            usage.pending,
            usage.udp_sessions,
            usage.queued_datagrams,
            usage.queued_bytes
        ),
        (1, 1, 2, 512)
    );
    assert_eq!(usage.dropped_datagrams, 3);
    assert!(matches!(
        budget.tcp_admission(&scope),
        Err(ProxyError::LocalResource)
    ));
    // Timing out a drain must reopen capture registration. Otherwise a later
    // start would remain rejected even though the owner scope is still live.
    assert!(
        tokio::time::timeout(Duration::from_millis(10), engine.wait_captured_idle())
            .await
            .is_err()
    );
    let fresh_probe = CaptureBudget::new(&specification).unwrap();
    let (_, reply, context) = endpoints().await;
    assert!(matches!(
        engine.spawn_captured_udp(
            reply,
            context,
            7,
            fresh_probe.udp_admission(&scope).unwrap()
        ),
        Err(ProxyError::LocalResource)
    ));

    // New configuration generations share the actual global connection gate,
    // even if an incorrect caller attempts to create a fresh capture budget.
    let next = Engine::with_capture(config.clone(), Some(&engine), capabilities).unwrap();
    next.set_generation(8);
    let fresh = CaptureBudget::new(&specification).unwrap();
    let (_, reply, context) = endpoints().await;
    assert!(matches!(
        next.spawn_captured_udp(reply, context, 8, fresh.udp_admission(&scope).unwrap()),
        Err(ProxyError::LocalResource)
    ));
    assert_eq!((fresh.usage().pending, fresh.usage().udp_sessions), (0, 0));

    relay.handshake.add_permits(1);
    let mut bytes = [0; 1024];
    let (length, source) =
        tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(length, 512);
    assert_eq!(&bytes[..length], &[9; 512]);
    let (empty, empty_source) =
        tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(empty, 0);
    assert_eq!(source, empty_source);
    tokio::time::timeout(Duration::from_secs(1), async {
        while budget.usage().pending != 0 || budget.usage().queued_datagrams != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        budget.udp_admission(&scope),
        Err(ProxyError::LocalResource)
    ));
    assert_eq!(budget.usage().pending, 0);
    drop(budget.tcp_admission(&scope).unwrap());
    assert_eq!(budget.usage().pending, 0);
    assert_eq!(engine.node_registry().connection_statistics().bytes_up, 512);
    assert_eq!(
        engine.node_registry().connection_statistics().bytes_down,
        512
    );

    session.try_send(b"late-reply").unwrap();
    tokio::time::timeout(Duration::from_secs(1), relay.late_seen.notified())
        .await
        .unwrap();
    session.cancel();
    scope.cancel();
    tokio::time::timeout(Duration::from_secs(1), engine.wait_captured_idle())
        .await
        .unwrap();
    let usage = budget.usage();
    assert_eq!(
        (
            usage.pending,
            usage.udp_sessions,
            usage.queued_datagrams,
            usage.queued_bytes
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(usage.closed_udp_sessions, 1);
    assert!(session.is_closed());
    relay.late.add_permits(1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), client.recv(&mut bytes))
            .await
            .is_err()
    );
    assert!(engine.status().inbounds[0].running);
    let explicit = TcpStream::connect(engine.status().inbounds[0].listen.clone().unwrap())
        .await
        .unwrap();
    drop(explicit);
    assert!(matches!(
        budget.tcp_admission(&scope),
        Err(ProxyError::Disconnected)
    ));

    let new_scope = CancellationToken::new();
    let (client, reply, context) = endpoints().await;
    relay.handshake.add_permits(1);
    let idle = next
        .spawn_captured_udp(reply, context, 8, budget.udp_admission(&new_scope).unwrap())
        .unwrap();
    idle.try_send(b"idle-flow").unwrap();
    let length = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes[..length], b"idle-flow");
    tokio::time::timeout(Duration::from_secs(3), next.wait_captured_idle())
        .await
        .unwrap();
    assert!(idle.is_closed());
    assert_eq!(budget.usage().udp_sessions, 0);
    assert_eq!(budget.usage().closed_udp_sessions, 2);
    assert_eq!(relay.datagrams.load(Ordering::Relaxed), 4);
    next.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn closed_control_tcp_interrupts_udp_budget_wait_and_releases_every_permit() {
    let relay = relay(Behavior::CloseAfterAssociate).await;
    relay.handshake.add_permits(1);
    let budget = CaptureBudget::new(&settings()).unwrap();
    let scope = CancellationToken::new();
    let mut config = config(relay.address);
    config.inbounds.clear();
    config.limits.bytes_per_second = 1;
    let engine = Engine::with_capture(
        config,
        None,
        RoutingCapabilities {
            local_process: true,
            udp: true,
        },
    )
    .unwrap();
    engine.set_generation(1);
    let (_client, reply, context) = endpoints().await;
    let session = engine
        .spawn_captured_udp(reply, context, 1, budget.udp_admission(&scope).unwrap())
        .unwrap();
    session.try_send(&[8; 512]).unwrap();
    // Sending this packet would otherwise wait over 8 minutes at the configured
    // business rate. Closing the association must interrupt that pacing wait.
    tokio::time::timeout(Duration::from_secs(1), engine.wait_captured_idle())
        .await
        .unwrap();
    assert!(session.is_closed());
    assert_eq!(
        (
            budget.usage().pending,
            budget.usage().udp_sessions,
            budget.usage().queued_bytes
        ),
        (0, 0, 0)
    );
    assert_eq!(relay.datagrams.load(Ordering::Relaxed), 0);
    assert_eq!(engine.node_registry().active("socks"), 0);
    engine.shutdown().await;
}

#[tokio::test]
async fn stopping_capture_scope_cancels_a_pending_association_and_its_queued_packets() {
    let relay = relay(Behavior::Echo).await;
    let budget = CaptureBudget::new(&settings()).unwrap();
    let scope = CancellationToken::new();
    let mut config = config(relay.address);
    config.inbounds.clear();
    let engine = Engine::with_capture(
        config,
        None,
        RoutingCapabilities {
            local_process: true,
            udp: true,
        },
    )
    .unwrap();
    engine.set_generation(1);
    let (_client, reply, context) = endpoints().await;
    let session = engine
        .spawn_captured_udp(reply, context, 1, budget.udp_admission(&scope).unwrap())
        .unwrap();
    session.try_send(&[4; 512]).unwrap();
    scope.cancel();
    tokio::time::timeout(Duration::from_secs(1), engine.wait_captured_idle())
        .await
        .unwrap();
    assert!(session.is_closed());
    let usage = budget.usage();
    assert_eq!(
        (
            usage.pending,
            usage.udp_sessions,
            usage.queued_datagrams,
            usage.queued_bytes
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(relay.datagrams.load(Ordering::Relaxed), 0);
    engine.shutdown().await;
}

#[tokio::test]
async fn closed_control_tcp_also_interrupts_a_paced_reply_when_no_sender_is_polling_control() {
    let relay = relay(Behavior::LargeReplyThenClose).await;
    relay.handshake.add_permits(1);
    let budget = CaptureBudget::new(&settings()).unwrap();
    let scope = CancellationToken::new();
    let mut config = config(relay.address);
    config.inbounds.clear();
    config.limits.bytes_per_second = 10;
    let engine = Engine::with_capture(
        config,
        None,
        RoutingCapabilities {
            local_process: true,
            udp: true,
        },
    )
    .unwrap();
    engine.set_generation(1);
    let (client, reply, context) = endpoints().await;
    let session = engine
        .spawn_captured_udp(reply, context, 1, budget.udp_admission(&scope).unwrap())
        .unwrap();
    session.try_send(b"x").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while engine.node_registry().connection_statistics().bytes_down != 512 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // The incoming 512-byte reply now waits over 50 seconds in the rate budget;
    // no next request is queued. A separate control monitor must end the owner.
    relay.late.add_permits(1);
    tokio::time::timeout(Duration::from_secs(1), engine.wait_captured_idle())
        .await
        .unwrap();
    let mut bytes = [0; 1024];
    assert!(
        tokio::time::timeout(Duration::from_millis(100), client.recv(&mut bytes))
            .await
            .is_err()
    );
    assert!(session.is_closed());
    assert_eq!(
        (
            budget.usage().pending,
            budget.usage().udp_sessions,
            budget.usage().queued_bytes
        ),
        (0, 0, 0)
    );
    engine.shutdown().await;
}
