use hps_config::UpstreamCredentials;
use hps_engine::{
    ProxyError,
    target::Target,
    udp::{SocksUdpAssociation, decode_datagram, encode_datagram},
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    time::Instant,
};

struct Resolver {
    address: IpAddr,
    calls: Arc<AtomicUsize>,
}
impl hps_net::NameResolver for Resolver {
    fn lookup(
        &self,
        host: String,
        purpose: hps_net::ResolvePurpose,
    ) -> hps_net::NetworkFuture<'_, Vec<IpAddr>> {
        assert_eq!(host, "relay.test");
        assert_eq!(purpose, hps_net::ResolvePurpose::Bootstrap);
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move { Ok(vec![self.address]) })
    }
}
async fn greeting(control: &mut TcpStream, credentials: bool) {
    assert_eq!(control.read_u8().await.unwrap(), 5);
    let methods = control.read_u8().await.unwrap();
    let mut methods = vec![0; usize::from(methods)];
    control.read_exact(&mut methods).await.unwrap();
    assert!(methods.contains(&0));
    if credentials {
        assert!(methods.contains(&2));
        control.write_all(&[5, 2]).await.unwrap();
        assert_eq!(control.read_u8().await.unwrap(), 1);
        let length = usize::from(control.read_u8().await.unwrap());
        let mut username = vec![0; length];
        control.read_exact(&mut username).await.unwrap();
        let length = usize::from(control.read_u8().await.unwrap());
        let mut password = vec![0; length];
        control.read_exact(&mut password).await.unwrap();
        assert_eq!(username, b"vendor");
        assert_eq!(password, b"lease-secret");
        control.write_all(&[1, 0]).await.unwrap();
    } else {
        control.write_all(&[5, 0]).await.unwrap();
    }
}

#[tokio::test]
async fn actual_udp_associate_preserves_dual_stack_datagrams_filters_relay_and_ends_with_tcp() {
    for ip in ["127.0.0.1", "::1"] {
        for credentials in [false, true] {
            let ip = ip.parse::<IpAddr>().unwrap();
            let bind = SocketAddr::new(ip, 0);
            let origin = UdpSocket::bind(bind).await.unwrap();
            let destination = origin.local_addr().unwrap();
            let echo = tokio::spawn(async move {
                let mut bytes = [0; 128];
                let (length, peer) = origin.recv_from(&mut bytes).await.unwrap();
                assert_eq!(&bytes[..length], b"first-datagram");
                origin.send_to(b"actual-origin-reply", peer).await.unwrap();
            });
            let listener = TcpListener::bind(bind).await.unwrap();
            let endpoint = listener.local_addr().unwrap();
            let relay = UdpSocket::bind(bind).await.unwrap();
            let relay_address = relay.local_addr().unwrap();
            let relayed = Arc::new(AtomicUsize::new(0));
            let observed = relayed.clone();
            let (close_tx, close_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut control, _) = listener.accept().await.unwrap();
                greeting(&mut control, credentials).await;
                let mut prefix = [0; 4];
                control.read_exact(&mut prefix).await.unwrap();
                assert_eq!(prefix, [5, 3, 0, if ip.is_ipv4() { 1 } else { 4 }]);
                let mut unknown = vec![0; if ip.is_ipv4() { 6 } else { 18 }];
                control.read_exact(&mut unknown).await.unwrap();
                assert!(unknown.iter().all(|b| *b == 0));
                let mut reply = vec![5, 0, 0];
                if credentials {
                    reply.extend_from_slice(&[3, 10]);
                    reply.extend_from_slice(b"relay.test");
                } else {
                    reply.push(if ip.is_ipv4() { 1 } else { 4 });
                    reply.resize(if ip.is_ipv4() { 8 } else { 20 }, 0);
                }
                reply.extend_from_slice(&relay_address.port().to_be_bytes());
                control.write_all(&reply).await.unwrap();
                let mut bytes = vec![0; 65536];
                let (length, client) = relay.recv_from(&mut bytes).await.unwrap();
                let (target, payload) = decode_datagram(&bytes[..length]).unwrap();
                assert_eq!(
                    target,
                    Target::new(&destination.ip().to_string(), destination.port()).unwrap()
                );
                let outgoing = UdpSocket::bind(bind).await.unwrap();
                outgoing.connect(destination).await.unwrap();
                outgoing.send(payload).await.unwrap();
                let mut response = [0; 128];
                let length = outgoing.recv(&mut response).await.unwrap();
                observed.fetch_add(1, Ordering::Relaxed);
                let actual = encode_datagram(&target, &response[..length]).unwrap();
                let stranger = UdpSocket::bind(bind).await.unwrap();
                stranger
                    .send_to(&encode_datagram(&target, b"wrong-relay").unwrap(), client)
                    .await
                    .unwrap();
                let mut fragmented = actual.clone();
                fragmented[2] = 1;
                relay.send_to(&fragmented, client).await.unwrap();
                relay.send_to(&actual, client).await.unwrap();
                close_rx.await.unwrap();
            });
            let calls = Arc::new(AtomicUsize::new(0));
            let resolver: Arc<dyn hps_net::NameResolver> = Arc::new(Resolver {
                address: ip,
                calls: calls.clone(),
            });
            let auth = credentials.then_some(UpstreamCredentials {
                username: "vendor".into(),
                password: "lease-secret".into(),
            });
            let association = SocksUdpAssociation::associate(
                TcpStream::connect(endpoint).await.unwrap(),
                auth.as_ref(),
                Some(resolver),
                65507,
                Instant::now() + Duration::from_secs(2),
            )
            .await
            .unwrap();
            let target = Target::new(&destination.ip().to_string(), destination.port()).unwrap();
            assert_eq!(
                association.send(&target, b"first-datagram").await.unwrap(),
                14
            );
            let mut buffer = [0; 128];
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(2), association.recv(&mut buffer))
                    .await
                    .unwrap(),
                Err(ProxyError::Protocol)
            ));
            let (length, source) =
                tokio::time::timeout(Duration::from_secs(2), association.recv(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(source, target);
            assert_eq!(&buffer[..length], b"actual-origin-reply");
            assert!(matches!(
                association.send(&target, &vec![0; 65507]).await,
                Err(ProxyError::Protocol)
            ));
            assert_eq!(calls.load(Ordering::Relaxed), usize::from(credentials));
            assert_eq!(relayed.load(Ordering::Relaxed), 1);
            close_tx.send(()).unwrap();
            server.await.unwrap();
            echo.await.unwrap();
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(2), association.recv(&mut buffer))
                    .await
                    .unwrap(),
                Err(ProxyError::Disconnected)
            ));
            assert!(matches!(
                association.send(&target, b"must-not-replay").await,
                Err(ProxyError::Disconnected)
            ));
        }
    }
}

#[tokio::test]
async fn udp_associate_timeout_closes_control_without_retrying_business_data() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut control, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 3];
        control.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 1, 0]);
        assert_eq!(control.read(&mut [0; 1]).await.unwrap(), 0);
    });
    let outcome = SocksUdpAssociation::associate(
        TcpStream::connect(endpoint).await.unwrap(),
        None,
        None,
        1024,
        Instant::now() + Duration::from_millis(50),
    )
    .await;
    assert!(matches!(outcome, Err(ProxyError::Timeout)));
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn upstream_authentication_and_udp_command_rejection_never_become_an_open_association() {
    for reject_auth in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut control, _) = listener.accept().await.unwrap();
            if reject_auth {
                let mut methods = [0; 4];
                control.read_exact(&mut methods).await.unwrap();
                assert_eq!(methods, [5, 2, 0, 2]);
                control.write_all(&[5, 2]).await.unwrap();
                let mut credentials = [0; 21];
                control.read_exact(&mut credentials).await.unwrap();
                assert_eq!(credentials[0], 1);
                control.write_all(&[1, 1]).await.unwrap();
            } else {
                greeting(&mut control, false).await;
                let mut command = [0; 10];
                control.read_exact(&mut command).await.unwrap();
                assert_eq!(command[..4], [5, 3, 0, 1]);
                control
                    .write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
            }
        });
        let credentials = UpstreamCredentials {
            username: "vendor".into(),
            password: "lease-secret".into(),
        };
        let outcome = SocksUdpAssociation::associate(
            TcpStream::connect(endpoint).await.unwrap(),
            reject_auth.then_some(&credentials),
            None,
            1024,
            Instant::now() + Duration::from_secs(2),
        )
        .await;
        if reject_auth {
            assert!(matches!(outcome, Err(ProxyError::Authentication)));
        } else {
            assert!(matches!(outcome, Err(ProxyError::TargetConnect)));
        }
        server.await.unwrap();
    }
}
