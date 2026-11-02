mod support;
use hps_config::{ProxyConfig, ProxyProtocol};
use hps_domain::RouteAction;
use hps_engine::{
    runtime::Engine,
    target::Target,
    udp::{decode_datagram, encode_datagram},
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

async fn engine(ipv6: bool, upstream: Option<SocketAddr>, direct: bool) -> Arc<Engine> {
    let mut inbound = support::inbound("socks", ProxyProtocol::Socks5, None);
    inbound.listen = if ipv6 { "[::1]:0" } else { "127.0.0.1:0" }
        .parse()
        .unwrap();
    let mut config = ProxyConfig {
        inbounds: vec![inbound],
        ..Default::default()
    };
    config.limits.min_remaining_lifetime_secs = 0;
    if direct {
        config.routing.default_action = RouteAction::Direct;
    }
    if let Some(upstream) = upstream {
        config
            .upstreams
            .push(support::upstream(upstream, ProxyProtocol::Socks5));
    }
    let engine = Engine::new(config).unwrap();
    engine.start_enabled().await.unwrap();
    engine
}
fn address(engine: &Engine) -> SocketAddr {
    engine.status().inbounds[0]
        .listen
        .as_ref()
        .unwrap()
        .parse()
        .unwrap()
}
async fn associate(engine: &Engine, requested: SocketAddr) -> (TcpStream, SocketAddr, u8) {
    let mut tcp = TcpStream::connect(address(engine)).await.unwrap();
    tcp.write_all(&[5, 1, 0]).await.unwrap();
    let mut greeting = [0; 2];
    tcp.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 0]);
    let mut request = vec![5, 3, 0];
    let host = requested.ip().to_string();
    request.extend(
        encode_datagram(&Target::new(&host, requested.port().max(1)).unwrap(), b"").unwrap()[3..]
            .iter(),
    );
    let len = request.len();
    request[len - 2..].copy_from_slice(&requested.port().to_be_bytes());
    tcp.write_all(&request).await.unwrap();
    let mut prefix = [0; 4];
    tcp.read_exact(&mut prefix).await.unwrap();
    let ip = match prefix[3] {
        1 => {
            let mut b = [0; 4];
            tcp.read_exact(&mut b).await.unwrap();
            std::net::IpAddr::from(b)
        }
        4 => {
            let mut b = [0; 16];
            tcp.read_exact(&mut b).await.unwrap();
            std::net::IpAddr::from(b)
        }
        _ => panic!("unexpected reply"),
    };
    let port = tcp.read_u16().await.unwrap();
    (tcp, SocketAddr::new(ip, port), prefix[1])
}
async fn echo(ipv6: bool) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let socket = UdpSocket::bind(if ipv6 { "[::1]:0" } else { "127.0.0.1:0" })
        .await
        .unwrap();
    let address = socket.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut b = vec![0; 65536];
        while let Ok((n, p)) = socket.recv_from(&mut b).await {
            socket.send_to(&b[..n], p).await.unwrap();
        }
    });
    (address, task)
}
#[tokio::test]
async fn udp_associate_dual_stack_and_socks_upstream_use_real_datagrams() {
    for ipv6 in [false, true] {
        let exit = engine(ipv6, None, true).await;
        let entry = engine(ipv6, Some(address(&exit)), false).await;
        let (origin, task) = echo(ipv6).await;
        let socket = UdpSocket::bind(if ipv6 { "[::1]:0" } else { "127.0.0.1:0" })
            .await
            .unwrap();
        let (tcp, relay, code) = associate(&entry, socket.local_addr().unwrap()).await;
        assert_eq!(code, 0);
        let target = Target::new(&origin.ip().to_string(), origin.port()).unwrap();
        for payload in [b"real-socks-udp".as_slice(), b"".as_slice()] {
            socket
                .send_to(&encode_datagram(&target, payload).unwrap(), relay)
                .await
                .unwrap();
            let mut packet = vec![0; 65536];
            let (n, source) =
                tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut packet))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(source, relay);
            let (reported, body) = decode_datagram(&packet[..n]).unwrap();
            assert_eq!(reported, target);
            assert_eq!(body, payload);
        }
        drop(tcp);
        tokio::time::timeout(Duration::from_secs(3), async {
            while entry.status().active_connections != 0 || exit.status().active_connections != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Closing the owner TCP releases its UDP port and every target admission.
        let rebound = UdpSocket::bind(relay).await.unwrap();
        drop(rebound);
        entry.shutdown().await;
        exit.shutdown().await;
        task.abort();
    }
}
#[tokio::test]
async fn udp_source_port_and_frag_are_enforced_and_malformed_does_not_lock_port() {
    let entry = engine(false, None, true).await;
    let (origin, task) = echo(false).await;
    let good = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let bad = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (tcp, relay, code) = associate(&entry, "0.0.0.0:0".parse().unwrap()).await;
    assert_eq!(code, 0);
    let target = Target::new(&origin.ip().to_string(), origin.port()).unwrap();
    let frame = encode_datagram(&target, b"valid").unwrap();
    let mut frag = frame.clone();
    frag[2] = 1;
    bad.send_to(&frag, relay).await.unwrap();
    good.send_to(&frame, relay).await.unwrap();
    let mut b = [0; 100];
    tokio::time::timeout(Duration::from_secs(3), good.recv(&mut b))
        .await
        .unwrap()
        .unwrap();
    bad.send_to(&frame, relay).await.unwrap();
    good.send_to(&frag, relay).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(80), bad.recv(&mut b))
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(80), good.recv(&mut b))
            .await
            .is_err()
    );
    drop(tcp);
    entry.shutdown().await;
    task.abort();
}
#[tokio::test]
async fn udp_associate_rejects_foreign_source_and_no_udp_capability() {
    let entry = engine(false, None, true).await;
    let (_, _, code) = associate(&entry, "127.0.0.2:9999".parse().unwrap()).await;
    assert_eq!(code, 2);
    entry.shutdown().await;
    let entry = engine(false, None, false).await;
    let (_, _, code) = associate(&entry, "0.0.0.0:0".parse().unwrap()).await;
    assert_eq!(code, 7);
    entry.shutdown().await;
}

#[tokio::test]
async fn udp_targets_share_global_entry_limits_and_inbound_stop_releases_all() {
    let mut config = ProxyConfig::default();
    let mut inbound = support::inbound("socks", ProxyProtocol::Socks5, None);
    inbound.max_connections = 3;
    config.inbounds.push(inbound);
    config.limits.max_connections = 4;
    config.routing.default_action = RouteAction::Direct;
    let entry = Engine::new(config).unwrap();
    entry.start_enabled().await.unwrap();
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (tcp, relay, code) = associate(&entry, socket.local_addr().unwrap()).await;
    assert_eq!(code, 0);
    let mut origins = Vec::new();
    let mut tasks = Vec::new();
    for _ in 0..3 {
        let (origin, task) = echo(false).await;
        origins.push(origin);
        tasks.push(task);
    }
    let mut buffer = [0; 200];
    for origin in origins.iter().take(2) {
        let target = Target::new(&origin.ip().to_string(), origin.port()).unwrap();
        socket
            .send_to(&encode_datagram(&target, b"admitted").unwrap(), relay)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), socket.recv(&mut buffer))
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(entry.status().active_connections, 3); // one TCP owner, two target flows
    let third = Target::new(&origins[2].ip().to_string(), origins[2].port()).unwrap();
    socket
        .send_to(&encode_datagram(&third, b"over-budget").unwrap(), relay)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(80), socket.recv(&mut buffer))
            .await
            .is_err()
    );
    assert_eq!(entry.status().active_connections, 3);
    entry.stop("socks").await.unwrap();
    assert_eq!(entry.status().active_connections, 0);
    let rebound = UdpSocket::bind(relay).await.unwrap();
    drop(rebound);
    drop(tcp);
    for task in tasks {
        task.abort();
    }
    entry.shutdown().await;
}
