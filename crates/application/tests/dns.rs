use hickory_proto::{
    op::{Message, OpCode},
    rr::{RData, Record, rdata::A},
};
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{DaemonConfig, DnsProtocol, DnsSave};
use hps_dns::{LookupInput, wire};
use hps_net::ResolvePurpose;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};

#[tokio::test]
async fn dns_udp_tcp_hot_policy_cas_bind_failure_and_shutdown_are_real() {
    let upstream = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let address = upstream.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut buffer = vec![0; 65535];
        loop {
            let (len, peer) = upstream.recv_from(&mut buffer).await.unwrap();
            let query = Message::from_vec(&buffer[..len]).unwrap();
            let mut response = Message::response(query.metadata.id, OpCode::Query);
            response.answers.push(Record::from_rdata(
                query.queries[0].name().clone(),
                30,
                RData::A(A::new(203, 0, 113, 9)),
            ));
            response.queries = query.queries;
            upstream
                .send_to(&response.to_vec().unwrap(), peer)
                .await
                .unwrap();
        }
    });
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.dns.upstreams[0].protocol = DnsProtocol::Udp;
    config.dns.upstreams[0].addresses = vec![address];
    config.dns.listen = Some("127.0.0.1:0".parse().unwrap());
    let service = Supervisor::boot(
        hps_storage::Store::open(None, config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    service.confirm_startup().await.unwrap();
    let view = service.dns_overview(None, 50).unwrap();
    let listener = view.listener.unwrap();
    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let query = wire::query("from-listener.example", "A").unwrap();
    let bytes = query.to_vec().unwrap();
    udp.send_to(&bytes, listener).await.unwrap();
    let mut buffer = vec![0; 65535];
    let len = tokio::time::timeout(Duration::from_secs(2), udp.recv(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    let response = Message::from_vec(&buffer[..len]).unwrap();
    assert_eq!(response.metadata.id, query.metadata.id);
    assert_eq!(
        wire::addresses(&response),
        vec!["203.0.113.9".parse::<std::net::IpAddr>().unwrap()]
    );
    let mut tcp = TcpStream::connect(listener).await.unwrap();
    async fn query_tcp(tcp: &mut TcpStream, wire: &[u8]) -> Message {
        tcp.write_u16(wire.len() as u16).await.unwrap();
        tcp.write_all(wire).await.unwrap();
        let len = tcp.read_u16().await.unwrap();
        let mut buffer = vec![0; len as usize];
        tcp.read_exact(&mut buffer).await.unwrap();
        Message::from_vec(&buffer).unwrap()
    }
    assert!(!query_tcp(&mut tcp, &bytes).await.answers.is_empty());
    let old = service.current();
    let mut unchanged = view.configuration.clone();
    unchanged.cache_entries = 10;
    let saved = service
        .edit_dns(DnsSave {
            configuration: unchanged.clone(),
            expected_revision: 1,
        })
        .await
        .unwrap();
    assert_eq!(saved.running_revision, 2);
    assert_eq!(
        service.dns_overview(None, 50).unwrap().listener,
        Some(listener)
    );
    assert_ne!(old.dns.fingerprint(), service.current().dns.fingerprint());
    assert!(!query_tcp(&mut tcp, &bytes).await.answers.is_empty());
    assert_eq!(service.dns_overview(None, 50).unwrap().cache.total, 1); // same TCP connection uses the new resolver
    assert!(matches!(
        service
            .edit_dns(DnsSave {
                configuration: unchanged.clone(),
                expected_revision: 1
            })
            .await,
        Err(hps_application::ApplicationError::Storage(
            hps_storage::StorageError::Conflict
        ))
    ));
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    unchanged.listen = Some(occupied.local_addr().unwrap());
    assert!(matches!(
        service
            .edit_dns(DnsSave {
                configuration: unchanged,
                expected_revision: 2
            })
            .await,
        Err(hps_application::ApplicationError::Engine(
            hps_engine::runtime::EngineError::Bind
        ))
    ));
    assert_eq!(service.workspace().await.unwrap().desired_revision, 2);
    assert_eq!(
        service.dns_overview(None, 50).unwrap().listener,
        Some(listener)
    );
    assert_eq!(
        service
            .current()
            .dns
            .diagnose(LookupInput {
                name: "diagnostic.example".into(),
                query_type: "A".into(),
                purpose: ResolvePurpose::Health
            })
            .await
            .unwrap()
            .upstream,
        "system"
    );
    drop(tcp);
    service.shutdown().await.unwrap();
    assert!(TcpStream::connect(listener).await.is_err());
    task.abort();
}

#[tokio::test]
async fn dns_static_egress_cannot_reenter_its_own_proxy_inlet() {
    use hps_config::{InboundConfig, ProxyProtocol, UpstreamConfig};
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let inlet = reservation.local_addr().unwrap();
    drop(reservation);
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.proxy.inbounds = vec![InboundConfig {
        id: "local".into(),
        protocol: ProxyProtocol::Http,
        listen: inlet,
        enabled: true,
        tls: None,
        max_connections: 8,
        bytes_per_second: 0,
    }];
    config.proxy.upstreams = vec![UpstreamConfig {
        id: "self".into(),
        protocol: ProxyProtocol::Http,
        host: "proxy.invalid".into(),
        port: inlet.port(),
        enabled: true,
        credentials: None,
        expires_at_unix: None,
        ca_certificate: None,
        tls_server_name: None,
    }];
    let upstream = &mut config.dns.upstreams[0];
    upstream.protocol = DnsProtocol::Tcp;
    upstream.addresses = vec!["203.0.113.53:53".parse().unwrap()];
    upstream.proxy_id = Some("self".into());
    upstream.proxy_bootstrap_ips = vec![inlet.ip()];
    let store = hps_storage::Store::open(None, config).unwrap();
    assert!(matches!(
        Supervisor::boot(store, StartupOverrides::default()).await,
        Err(hps_application::ApplicationError::Invalid)
    ));
    let _unused = TcpListener::bind(inlet).await.unwrap();
}
