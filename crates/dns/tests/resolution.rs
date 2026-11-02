use hickory_proto::{
    op::{Message, OpCode, ResponseCode},
    rr::{
        Name, RData, Record, RecordType,
        rdata::{A, AAAA, CNAME, SOA},
    },
};
use hps_config::{DnsConfig, DnsProtocol, DnsRoute, ProxyConfig};
use hps_dns::{DnsError, LookupInput, Resolver, wire};
use hps_net::{NameResolver, ResolvePurpose};
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
    net::{TcpListener, UdpSocket},
};

fn answer(query: &Message, ttl: u32) -> Message {
    let q = &query.queries[0];
    let mut response = Message::response(query.metadata.id, OpCode::Query);
    response.queries = query.queries.clone();
    response.metadata.recursion_available = true;
    let name = q.name().to_ascii();
    if name.starts_with("negative.") {
        response.metadata.response_code = ResponseCode::NXDomain;
        response.authorities.push(Record::from_rdata(
            Name::from_ascii("example.").unwrap(),
            30,
            RData::SOA(SOA::new(
                Name::from_ascii("ns.example.").unwrap(),
                Name::from_ascii("host.example.").unwrap(),
                1,
                10,
                10,
                10,
                2,
            )),
        ));
    } else {
        let data = if q.query_type() == RecordType::AAAA {
            RData::AAAA(AAAA("2001:db8::9".parse().unwrap()))
        } else {
            RData::A(A::new(203, 0, 113, 9))
        };
        response.answers.push(Record::from_rdata(
            q.name().clone(),
            if name.starts_with("zero.") { 0 } else { ttl },
            data,
        ));
        if name.starts_with("large.") {
            for _ in 0..80 {
                response.answers.push(response.answers[0].clone());
            }
            let mut edns = hickory_proto::op::Edns::new();
            edns.options_mut()
                .insert(hickory_proto::rr::rdata::opt::EdnsOption::Unknown(
                    12,
                    vec![0; 2000],
                ));
            response.edns = Some(edns);
        }
    }
    response
}
struct Upstream {
    address: SocketAddr,
    count: Arc<AtomicUsize>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Upstream {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
impl Upstream {
    async fn start(ip: &str, ttl: u32, delay: Duration, truncated: bool) -> Self {
        let tcp = TcpListener::bind(format!("{ip}:0")).await.unwrap();
        let address = tcp.local_addr().unwrap();
        let udp = Arc::new(UdpSocket::bind(address).await.unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        let u = tokio::spawn(async move {
            let mut buf = vec![0; 65535];
            loop {
                let (size, peer) = udp.recv_from(&mut buf).await.unwrap();
                let query = Message::from_vec(&buf[..size]).unwrap();
                counter.fetch_add(1, Ordering::SeqCst);
                let socket = udp.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let mut response = answer(&query, ttl);
                    // The service must ignore a datagram with a wrong transaction ID.
                    let mut wrong = response.clone();
                    wrong.metadata.id = wrong.metadata.id.wrapping_add(1);
                    socket
                        .send_to(&wrong.to_vec().unwrap(), peer)
                        .await
                        .unwrap();
                    if truncated {
                        response.answers.clear();
                        response.metadata.truncation = true;
                    }
                    socket
                        .send_to(&response.to_vec().unwrap(), peer)
                        .await
                        .unwrap();
                });
            }
        });
        let counter = count.clone();
        let t = tokio::spawn(async move {
            loop {
                let (mut stream, _) = tcp.accept().await.unwrap();
                let counter = counter.clone();
                tokio::spawn(async move {
                    let len = stream.read_u16().await.unwrap();
                    let mut data = vec![0; len as usize];
                    stream.read_exact(&mut data).await.unwrap();
                    let query = Message::from_vec(&data).unwrap();
                    counter.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(delay).await;
                    let data = answer(&query, ttl).to_vec().unwrap();
                    stream.write_u16(data.len() as u16).await.unwrap();
                    stream.write_all(&data).await.unwrap();
                });
            }
        });
        Self {
            address,
            count,
            tasks: vec![u, t],
        }
    }
    fn resolver(&self, protocol: DnsProtocol) -> Resolver {
        let mut config = DnsConfig::default();
        config.upstreams[0].protocol = protocol;
        config.upstreams[0].addresses = vec![self.address];
        Resolver::prepare(config, &ProxyConfig::default(), None).unwrap()
    }
}
fn input(name: &str) -> LookupInput {
    LookupInput {
        name: name.into(),
        query_type: "A".into(),
        purpose: ResolvePurpose::Business,
    }
}

#[tokio::test]
async fn wire_ttl_zero_negative_and_real_ipv6_tcp_fallback() {
    let upstream = Upstream::start("127.0.0.1", 2, Duration::ZERO, false).await;
    let resolver = upstream.resolver(DnsProtocol::Udp);
    let first = resolver.diagnose(input("ttl.example")).await.unwrap();
    assert_eq!(first.answers[0].value, "203.0.113.9");
    assert!(first.remaining_ttl.unwrap() <= 2);
    assert_eq!(
        resolver
            .diagnose(input("TTL.EXAMPLE."))
            .await
            .unwrap()
            .source,
        "cache"
    );
    assert_eq!(upstream.count.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(1150)).await;
    let cached = resolver.diagnose(input("ttl.example")).await.unwrap();
    assert_eq!(cached.source, "cache");
    assert_eq!(cached.remaining_ttl, Some(0));
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(
        resolver
            .diagnose(input("ttl.example"))
            .await
            .unwrap()
            .source,
        "upstream"
    );
    for _ in 0..2 {
        assert_eq!(
            resolver
                .diagnose(input("zero.example"))
                .await
                .unwrap()
                .source,
            "upstream"
        );
    }
    let negative = resolver.diagnose(input("negative.example")).await.unwrap();
    assert_eq!(negative.response_code, "NXDomain");
    assert!(negative.authorities[0].ttl <= negative.remaining_ttl.unwrap());
    assert_eq!(
        resolver
            .diagnose(input("negative.example"))
            .await
            .unwrap()
            .source,
        "cache"
    );
    assert!(
        resolver
            .cache(None, 50)
            .unwrap()
            .entries
            .iter()
            .find(|e| e.name == "negative.example")
            .unwrap()
            .remaining_ttl
            <= 2
    );
    let upstream = Upstream::start("[::1]", 30, Duration::ZERO, true).await;
    let resolver = upstream.resolver(DnsProtocol::Udp);
    let ips = resolver
        .lookup("dual.example".into(), ResolvePurpose::Health)
        .await
        .unwrap();
    assert!(ips.contains(&"2001:db8::9".parse().unwrap()));
    assert_eq!(upstream.count.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn coalescing_survives_cancellation_clear_blocks_old_fill_and_capacity_is_bounded() {
    let upstream = Upstream::start("127.0.0.1", 30, Duration::from_millis(120), false).await;
    let mut config = DnsConfig::default();
    config.upstreams[0].protocol = DnsProtocol::Udp;
    config.upstreams[0].addresses = vec![upstream.address];
    config.max_parallel = 1;
    let resolver = Resolver::prepare(config, &ProxyConfig::default(), None).unwrap();
    let r = resolver.clone();
    let caller = tokio::spawn(async move { r.diagnose(input("owned.example")).await });
    while upstream.count.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        resolver.diagnose(input("other.example")).await.unwrap_err(),
        DnsError::Capacity
    );
    let r = resolver.clone();
    let merged = tokio::spawn(async move { r.diagnose(input("owned.example")).await.unwrap() });
    while resolver.cache(None, 50).unwrap().merged == 0 {
        tokio::task::yield_now().await;
    }
    caller.abort();
    resolver.clear(None).unwrap();
    assert_eq!(merged.await.unwrap().source, "merged");
    assert_eq!(upstream.count.load(Ordering::SeqCst), 1);
    assert_eq!(resolver.cache(None, 50).unwrap().total, 0);
    assert_eq!(
        resolver
            .diagnose(input("owned.example"))
            .await
            .unwrap()
            .source,
        "upstream"
    );
    assert_eq!(resolver.cache(None, 50).unwrap().total, 1);
}

#[tokio::test]
async fn route_purpose_edns_isolation_bounds_and_domain_ambiguity() {
    let upstream = Upstream::start("127.0.0.1", 30, Duration::ZERO, false).await;
    let mut config = DnsConfig::default();
    config.upstreams[0].protocol = DnsProtocol::Tcp;
    config.upstreams[0].addresses = vec![upstream.address];
    config.cache_entries = 3;
    let mut other = config.upstreams[0].clone();
    other.id = "other".into();
    config.upstreams.push(other);
    config.routes = vec![DnsRoute {
        id: "health-example".into(),
        upstream: "other".into(),
        domain: Some("example".into()),
        suffix: true,
        query_types: vec!["A".into()],
        purposes: vec![ResolvePurpose::Health],
    }];
    let resolver = Resolver::prepare(config, &ProxyConfig::default(), None).unwrap();
    resolver.diagnose(input("one.example")).await.unwrap();
    let mut health = input("one.example");
    health.purpose = ResolvePurpose::Health;
    assert_eq!(resolver.diagnose(health).await.unwrap().upstream, "other");
    let mut request = wire::query("one.example", "A").unwrap();
    request.metadata.checking_disabled = true;
    resolver
        .exchange(request, ResolvePurpose::Business)
        .await
        .unwrap();
    assert_eq!(resolver.cache(None, 50).unwrap().total, 3);
    assert_eq!(upstream.count.load(Ordering::SeqCst), 3);
    resolver.diagnose(input("two.example")).await.unwrap();
    let context = resolver.domain_context("203.0.113.9".parse().unwrap());
    assert!(context.ambiguous);
    assert_eq!(context.domains, vec!["one.example", "two.example"]);
    assert!(!context.mappings.is_empty() && !context.mappings_truncated);
    assert!(
        context
            .mappings
            .iter()
            .all(|m| m.source == "unexpired_local_dns_answers" && m.remaining_ttl <= 60)
    );
    assert_eq!(resolver.cache(None, 50).unwrap().total, 3);
    assert!(
        resolver
            .domain_context("192.0.2.100".parse().unwrap())
            .domains
            .is_empty()
    );
    assert_eq!(resolver.clear(Some("two.example")).unwrap(), 1);
    assert!(
        resolver
            .domain_context("203.0.113.9".parse().unwrap())
            .mappings
            .iter()
            .all(|m| m.domain != "two.example")
    );
    let page = resolver.cache(None, 1).unwrap();
    assert!(page.next_after.is_some());
    assert_eq!(
        resolver
            .cache(page.next_after.as_deref(), 1)
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[tokio::test]
async fn system_has_no_invented_ttl_and_timeouts_do_not_poison_cache() {
    let resolver = Resolver::prepare(DnsConfig::default(), &ProxyConfig::default(), None).unwrap();
    let response = resolver.diagnose(input("localhost")).await.unwrap();
    assert_eq!(response.remaining_ttl, None);
    assert_eq!(resolver.cache(None, 50).unwrap().total, 0);
    let mut txt = input("localhost");
    txt.query_type = "TXT".into();
    assert_eq!(
        resolver.diagnose(txt).await.unwrap_err(),
        DnsError::SystemQueryType
    );
    let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut config = DnsConfig {
        timeout_ms: 100,
        ..Default::default()
    };
    config.upstreams[0].protocol = DnsProtocol::Udp;
    config.upstreams[0].addresses = vec![sink.local_addr().unwrap()];
    let resolver = Resolver::prepare(config, &ProxyConfig::default(), None).unwrap();
    assert_eq!(
        resolver
            .diagnose(input("timeout.example"))
            .await
            .unwrap_err(),
        DnsError::Timeout
    );
    assert_eq!(resolver.cache(None, 50).unwrap().total, 0);
    assert_eq!(resolver.cache(None, 50).unwrap().in_flight, 0);
}

#[test]
fn cname_context_ignores_unrelated_answers_and_rejects_loops() {
    let query = wire::query("alias.example", "A").unwrap();
    let mut response = answer(&query, 30);
    response.answers.clear();
    response.answers.push(Record::from_rdata(
        Name::from_ascii("evil.example").unwrap(),
        30,
        RData::A(A::new(192, 0, 2, 1)),
    ));
    assert!(wire::addresses(&response).is_empty());
    response.answers.push(Record::from_rdata(
        Name::from_ascii("alias.example").unwrap(),
        20,
        RData::CNAME(CNAME(Name::from_ascii("target.example").unwrap())),
    ));
    response.answers.push(Record::from_rdata(
        Name::from_ascii("target.example").unwrap(),
        10,
        RData::A(A::new(203, 0, 113, 9)),
    ));
    assert_eq!(
        wire::addresses(&response),
        vec!["203.0.113.9".parse::<std::net::IpAddr>().unwrap()]
    );
    assert_eq!(wire::cache_ttl(&response), Some(10));
    response.metadata.response_code = ResponseCode::NXDomain;
    assert!(wire::addresses(&response).is_empty());
    response.metadata.response_code = ResponseCode::NoError;
    response.answers.pop();
    response.answers.push(Record::from_rdata(
        Name::from_ascii("target.example").unwrap(),
        10,
        RData::CNAME(CNAME(Name::from_ascii("alias.example").unwrap())),
    ));
    assert!(wire::addresses(&response).is_empty());
    assert_eq!(wire::cache_ttl(&response), None);
    let mut invalid = query.clone();
    invalid.answers = response.answers;
    assert_eq!(
        wire::validate_query(&invalid).unwrap_err(),
        DnsError::InvalidQuery
    );
}

#[tokio::test]
async fn generations_share_capacity_retire_caches_and_shutdown_cancels_owned_io() {
    let upstream = Upstream::start("127.0.0.1", 30, Duration::from_millis(150), false).await;
    let mut config = DnsConfig {
        max_parallel: 1,
        ..Default::default()
    };
    config.upstreams[0].protocol = DnsProtocol::Udp;
    config.upstreams[0].addresses = vec![upstream.address];
    let old = Resolver::prepare(config.clone(), &ProxyConfig::default(), None).unwrap();
    old.diagnose(input("cached.example")).await.unwrap();
    assert_eq!(old.cache(None, 50).unwrap().total, 1);
    let r = old.clone();
    let previous =
        tokio::spawn(async move { r.diagnose(input("old-flight.example")).await.unwrap() });
    while upstream.count.load(Ordering::SeqCst) < 2 {
        tokio::task::yield_now().await;
    }
    config.cache_entries = 10;
    let new = Resolver::with_previous(config, &ProxyConfig::default(), None, Some(&old)).unwrap();
    new.activate();
    old.retire();
    assert_eq!(new.cache(None, 50).unwrap().in_flight, 1);
    assert_eq!(
        new.diagnose(input("new-flight.example")).await.unwrap_err(),
        DnsError::Capacity
    );
    previous.await.unwrap();
    assert_eq!(old.cache(None, 50).unwrap().total, 0);
    assert_eq!(
        new.diagnose(input("new-flight.example"))
            .await
            .unwrap()
            .source,
        "upstream"
    );
    let r = new.clone();
    let owned = tokio::spawn(async move { r.diagnose(input("shutdown.example")).await });
    while upstream.count.load(Ordering::SeqCst) < 4 {
        tokio::task::yield_now().await;
    }
    new.shutdown().await;
    assert_eq!(owned.await.unwrap().unwrap_err(), DnsError::Worker);
    assert_eq!(new.cache(None, 50).unwrap().in_flight, 0);
    assert_eq!(
        new.diagnose(input("after-stop.example")).await.unwrap_err(),
        DnsError::Worker
    );
    old.shutdown().await;
}

#[tokio::test]
async fn server_truncates_oversized_edns_udp_and_tcp_returns_the_complete_answer() {
    let upstream = Upstream::start("127.0.0.1", 30, Duration::ZERO, false).await;
    let resolver = upstream.resolver(DnsProtocol::Udp);
    let server = hps_dns::server::DnsServer::bind("127.0.0.1:0".parse().unwrap(), resolver)
        .await
        .unwrap();
    server.start();
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let request = wire::query("large.example", "A").unwrap();
    let bytes = request.to_vec().unwrap();
    socket
        .send_to(&bytes, server.local_address())
        .await
        .unwrap();
    let mut buffer = vec![0; 65535];
    let len = tokio::time::timeout(Duration::from_secs(2), socket.recv(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    assert!(len <= 512);
    let response = Message::from_vec(&buffer[..len]).unwrap();
    assert!(response.metadata.truncation);
    assert_eq!(response.metadata.id, request.metadata.id);
    let mut tcp = tokio::net::TcpStream::connect(server.local_address())
        .await
        .unwrap();
    tcp.write_u16(bytes.len() as u16).await.unwrap();
    tcp.write_all(&bytes).await.unwrap();
    let len = tcp.read_u16().await.unwrap() as usize;
    tcp.read_exact(&mut buffer[..len]).await.unwrap();
    let response = Message::from_vec(&buffer[..len]).unwrap();
    assert!(!response.metadata.truncation);
    assert_eq!(response.answers.len(), 81);
    drop(tcp);
    server.shutdown().await;
}
