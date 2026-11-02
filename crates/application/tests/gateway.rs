#![cfg(target_os = "linux")]
#[path = "../../platform/src/transparent/test_support.rs"]
#[allow(
    dead_code,
    reason = "shared fixtures include unrelated local cgroup helpers"
)]
mod capture_support;
#[path = "../../platform/src/rtnetlink.rs"]
#[allow(
    dead_code,
    reason = "shared bounded RTNL fixture exposes extra inspection helpers"
)]
mod rtnetlink;
#[path = "../../platform/src/transparent/test_proxy.rs"]
mod test_proxy;
use capture_support::{Address, Jobs};
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{
    DaemonConfig, GroupConfig, GroupStrategy, InboundConfig, LinuxCaptureConfig, ProxyConfig,
    ProxyProtocol, RuleConditions, RuleConfig, UpstreamConfig,
};
use hps_domain::{FlowOrigin, RouteAction, SelectionTransport};
use std::{
    ffi::CString,
    io::{Read, Write},
    net::{IpAddr, SocketAddr},
    os::fd::AsRawFd,
    process::{Command, Stdio},
    sync::atomic::Ordering,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn index(name: &str) -> u32 {
    let value = unsafe { libc::if_nametoindex(CString::new(name).unwrap().as_ptr()) };
    assert_ne!(value, 0, "missing test link {name}");
    value
}
fn link_header(index: u32) -> Vec<u8> {
    let mut bytes = vec![0; 16];
    bytes[4..8].copy_from_slice(&index.to_ne_bytes());
    bytes
}
fn request(kind: u16, payload: &[u8], create: bool) {
    let flags = libc::NLM_F_REQUEST
        | libc::NLM_F_ACK
        | if create {
            libc::NLM_F_CREATE | libc::NLM_F_EXCL
        } else {
            0
        };
    rtnetlink::exchange(kind, flags as u16, payload, false).unwrap();
}
fn up(name: &str) {
    set_up(index(name), true);
}
fn set_up(index: u32, enabled: bool) {
    let mut payload = link_header(index);
    payload[8..12].copy_from_slice(&(if enabled { libc::IFF_UP as u32 } else { 0 }).to_ne_bytes());
    payload[12..16].copy_from_slice(&(libc::IFF_UP as u32).to_ne_bytes());
    request(libc::RTM_NEWLINK, &payload, false);
}
fn rename(index: u32, name: &str) {
    let mut payload = link_header(index);
    let name = CString::new(name).unwrap();
    rtnetlink::attribute(&mut payload, 3, name.as_bytes_with_nul());
    request(libc::RTM_NEWLINK, &payload, false);
}
fn peer_up(namespace: &std::fs::File, enabled: bool) {
    let namespace = namespace.try_clone().unwrap();
    std::thread::spawn(move || {
        assert_eq!(
            unsafe { libc::setns(namespace.as_raw_fd(), libc::CLONE_NEWNET) },
            0
        );
        set_up(index("hpsclient0"), enabled);
    })
    .join()
    .unwrap();
}
async fn suspended(service: &std::sync::Arc<Supervisor>, links_expected: bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = service.linux_capture_status().await;
            if status.suspended
                && !status.ready
                && status.links_present == Some(links_expected)
                && status.interfaces_ready == Some(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
}
async fn recheck(service: &std::sync::Arc<Supervisor>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while service.linux_capture_status().await.interfaces_ready != Some(true) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let status = service
        .control_linux_capture(hps_domain::LinuxCaptureRequest {
            expected_epoch: service.event_epoch().into(),
            expected_revision: 1,
            action: hps_domain::LinuxCaptureAction::Recheck,
        })
        .await
        .unwrap();
    assert!(status.ready, "{status:?}");
}
fn address(name: &str, ip: &str) {
    let ip = ip.parse::<IpAddr>().unwrap();
    let (family, prefix, bytes) = match ip {
        IpAddr::V4(ip) => (libc::AF_INET, 24, ip.octets().to_vec()),
        IpAddr::V6(ip) => (libc::AF_INET6, 64, ip.octets().to_vec()),
    };
    let mut payload = vec![family as u8, prefix, if ip.is_ipv6() { 2 } else { 0 }, 0];
    payload.extend_from_slice(&index(name).to_ne_bytes());
    rtnetlink::attribute(&mut payload, 1, &bytes);
    rtnetlink::attribute(&mut payload, 2, &bytes);
    request(libc::RTM_NEWADDR, &payload, true);
}
fn default_route(gateway: &str) {
    let ip = gateway.parse::<IpAddr>().unwrap();
    let (family, bytes) = match ip {
        IpAddr::V4(ip) => (libc::AF_INET, ip.octets().to_vec()),
        IpAddr::V6(ip) => (libc::AF_INET6, ip.octets().to_vec()),
    };
    let mut payload = vec![family as u8, 0, 0, 0, 254, 4, 0, 1, 0, 0, 0, 0];
    rtnetlink::attribute(&mut payload, 5, &bytes);
    rtnetlink::attribute(&mut payload, 4, &index("hpsclient0").to_ne_bytes());
    request(libc::RTM_NEWROUTE, &payload, true);
}
struct Veth(u32);
impl Veth {
    fn create(namespace: &std::fs::File) -> Self {
        let mut peer = link_header(0);
        rtnetlink::attribute(&mut peer, 3, b"hpsclient0\0");
        let mut data = vec![];
        rtnetlink::attribute(&mut data, 1 | 0x8000, &peer);
        let mut info = vec![];
        rtnetlink::attribute(&mut info, 1, b"veth\0");
        rtnetlink::attribute(&mut info, 2 | 0x8000, &data);
        let mut payload = link_header(0);
        rtnetlink::attribute(&mut payload, 3, b"hpslan0\0");
        rtnetlink::attribute(&mut payload, 18 | 0x8000, &info);
        request(libc::RTM_NEWLINK, &payload, true);
        let result = Self(index("hpslan0"));
        let mut payload = link_header(index("hpsclient0"));
        rtnetlink::attribute(&mut payload, 28, &namespace.as_raw_fd().to_ne_bytes());
        request(libc::RTM_NEWLINK, &payload, false);
        up("hpslan0");
        address("hpslan0", "10.231.83.1");
        address("hpslan0", "fd42:6870:7300:1::1");
        result
    }
}
impl Drop for Veth {
    fn drop(&mut self) {
        let _ = rtnetlink::exchange(
            libc::RTM_DELLINK,
            (libc::NLM_F_REQUEST | libc::NLM_F_ACK) as u16,
            &link_header(self.0),
            false,
        );
    }
}
#[test]
#[ignore = "helper invoked only in a child network namespace by real_gateway_shared_engine"]
fn gateway_child() {
    assert_eq!(unsafe { libc::unshare(libc::CLONE_NEWNET) }, 0);
    let mut control =
        std::os::unix::net::UnixStream::connect(std::env::var("HPS_GATEWAY_CONTROL").unwrap())
            .unwrap();
    control
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    control.write_all(b"R").unwrap();
    control
        .write_all(&(unsafe { libc::syscall(libc::SYS_gettid) } as u32).to_ne_bytes())
        .unwrap();
    let mut signal = [0];
    control.read_exact(&mut signal).unwrap();
    assert_eq!(signal, [b'P']);
    up("lo");
    up("hpsclient0");
    address("hpsclient0", "10.231.83.2");
    address("hpsclient0", "fd42:6870:7300:1::2");
    default_route("10.231.83.1");
    default_route("fd42:6870:7300:1::1");
    control.write_all(b"I").unwrap();
    control.read_exact(&mut signal).unwrap();
    assert_eq!(signal, [b'N']);
    let targets: Vec<SocketAddr> =
        serde_json::from_str(&std::env::var("HPS_GATEWAY_TARGETS").unwrap()).unwrap();
    assert_eq!(targets.len(), 6);
    for (i, target) in targets.into_iter().enumerate() {
        if !(2..4).contains(&i) {
            let mut stream =
                std::net::TcpStream::connect_timeout(&target, Duration::from_secs(5)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let body = vec![if i < 2 { b'T' } else { b'B' }; if i < 2 { 262144 } else { 32 }];
            stream.write_all(&body).unwrap();
            let mut reply = vec![0; body.len()];
            stream.read_exact(&mut reply).unwrap();
            assert_eq!(reply, body);
        } else {
            let socket = std::net::UdpSocket::bind(if target.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            })
            .unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let body = [b'U'; 1200];
            socket.send_to(&body, target).unwrap();
            let mut reply = [0; 2048];
            let (n, source) = socket.recv_from(&mut reply).unwrap();
            assert_eq!(source, target);
            assert_eq!(&reply[..n], body);
        }
    }
    control.write_all(b"D").unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux, veth, child netns and actual eBPF"]
async fn real_gateway_shared_engine() {
    let _addresses = [Address::add("203.0.113.77"), Address::add("2001:db8::77")];
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("gateway.sock");
    let control = tokio::net::UnixListener::bind(&path).unwrap();
    let mut jobs = Jobs(vec![]);
    let mut targets = vec![];
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.dns.enabled = false;
    config.proxy.routing.default_action = RouteAction::Reject;
    let mut upstream_config = ProxyConfig::default();
    upstream_config.routing.default_action = RouteAction::Direct;
    upstream_config.inbounds.push(InboundConfig {
        id: "gateway-http".into(),
        protocol: ProxyProtocol::Http,
        listen: "127.0.0.1:0".parse().unwrap(),
        enabled: true,
        tls: None,
        max_connections: 16,
        bytes_per_second: 0,
    });
    let upstream = hps_engine::runtime::Engine::new(upstream_config).unwrap();
    upstream.set_generation(1);
    upstream.start_enabled().await.unwrap();
    let http: SocketAddr = upstream.status().inbounds[0]
        .listen
        .as_ref()
        .unwrap()
        .parse()
        .unwrap();
    let mut socks = test_proxy::Proxy::bind().await;
    for (id, protocol, address, transport) in [
        ("http", ProxyProtocol::Http, http, SelectionTransport::Tcp),
        (
            "socks",
            ProxyProtocol::Socks5,
            socks.address,
            SelectionTransport::Udp,
        ),
    ] {
        config.proxy.upstreams.push(UpstreamConfig {
            id: id.into(),
            protocol,
            host: address.ip().to_string(),
            port: address.port(),
            enabled: true,
            credentials: None,
            expires_at_unix: None,
            ca_certificate: None,
            tls_server_name: None,
        });
        config.proxy.selection.groups.push(GroupConfig {
            id: id.into(),
            strategy: GroupStrategy::Fixed,
            members: vec![id.into()],
            pinned_node: Some(id.into()),
            ..Default::default()
        });
        config.proxy.routing.rules.push(RuleConfig {
            id: id.into(),
            enabled: true,
            conditions: RuleConditions {
                source_cidrs: vec!["10.231.83.0/24".into(), "fd42:6870:7300:1::/64".into()],
                transports: vec![transport],
                ..Default::default()
            },
            action: RouteAction::Group { id: id.into() },
        });
    }
    for ip in ["203.0.113.77", "2001:db8::77"] {
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(ip.parse().unwrap(), 0))
            .await
            .unwrap();
        targets.push(listener.local_addr().unwrap());
        jobs.0.push(tokio::spawn(async move {
            let (mut stream, peer) = listener.accept().await.unwrap();
            assert!(peer.ip().is_loopback() || peer.ip().to_string() == ip);
            let mut body = vec![0; 262144];
            stream.read_exact(&mut body).await.unwrap();
            assert!(body.iter().all(|b| *b == b'T'));
            stream.write_all(&body).await.unwrap();
        }));
    }
    for ip in ["203.0.113.77", "2001:db8::77"] {
        let socket = hps_net::transparent::bind_udp(
            SocketAddr::new(ip.parse().unwrap(), 0),
            LinuxCaptureConfig::default().bypass_mark,
        )
        .unwrap();
        targets.push(socket.local_addr().unwrap());
        jobs.0.push(tokio::spawn(async move {
            let mut body = [0; 2048];
            let (n, peer) = socket.recv_from(&mut body).await.unwrap();
            assert!(peer.ip().is_loopback() || peer.ip().to_string() == ip);
            assert_eq!(n, 1200);
            socket.send_to(&body[..n], peer).await.unwrap();
        }));
    }
    let mut bypass_ports = vec![];
    for ip in ["203.0.113.77", "2001:db8::77"] {
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(ip.parse().unwrap(), 0))
            .await
            .unwrap();
        let target = listener.local_addr().unwrap();
        targets.push(target);
        bypass_ports.push(target.port());
        jobs.0.push(tokio::spawn(async move {
            let (mut stream, peer) = listener.accept().await.unwrap();
            assert_eq!(
                peer.ip().to_string(),
                if peer.is_ipv4() {
                    "10.231.83.2"
                } else {
                    "fd42:6870:7300:1::2"
                }
            );
            let mut bytes = [0; 32];
            stream.read_exact(&mut bytes).await.unwrap();
            assert_eq!(bytes, [b'B'; 32]);
            stream.write_all(&bytes).await.unwrap();
        }));
    }
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "gateway_child", "--ignored", "--nocapture"])
        .env("HPS_GATEWAY_CONTROL", &path)
        .env(
            "HPS_GATEWAY_TARGETS",
            serde_json::to_string(&targets).unwrap(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (mut control, _) = tokio::time::timeout(Duration::from_secs(5), control.accept())
        .await
        .unwrap()
        .unwrap();
    let mut signal = [0];
    control.read_exact(&mut signal).await.unwrap();
    assert_eq!(signal, [b'R']);
    let mut tid = [0; 4];
    control.read_exact(&mut tid).await.unwrap();
    let namespace = std::fs::File::open(format!(
        "/proc/{}/task/{}/ns/net",
        child.id(),
        u32::from_ne_bytes(tid)
    ))
    .unwrap();
    let _veth = Veth::create(&namespace);
    control.write_all(b"P").await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), control.read_exact(&mut signal))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(signal, [b'I']);
    config.linux_capture = Some(LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: None,
        lan_interfaces: vec!["hpslan0".into()],
        tcp_port: 0,
        udp_port: 0,
        bypass_ports,
        max_flows: 64,
        max_pending: 16,
        max_udp_sessions: 16,
        ..Default::default()
    });
    let service = Supervisor::boot(
        hps_storage::Store::open(None, config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    service.confirm_startup().await.unwrap();
    assert!(service.linux_capture_status().await.ready);
    let rules = service.rules_overview(None).await.unwrap().capabilities;
    assert!(!rules.local_process_available);
    assert!(rules.udp_data_plane_available && rules.kernel_rules_ready);
    assert_eq!(rules.kernel_rule_revision, Some(1));
    control.write_all(b"N").await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(15), control.read_exact(&mut signal)).await;
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    if !output.status.success() {
        eprintln!(
            "gateway diagnostic status={:?} records={:?} associations={} relayed={}",
            service.linux_capture_status().await,
            service
                .current()
                .engine
                .node_registry()
                .connection_records(true),
            socks.associations.load(Ordering::Relaxed),
            socks.datagrams.load(Ordering::Relaxed)
        );
    }
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    done.unwrap().unwrap();
    assert_eq!(signal, [b'D']);
    for job in std::mem::take(&mut jobs.0) {
        tokio::time::timeout(Duration::from_secs(3), job)
            .await
            .unwrap()
            .unwrap();
    }
    let registry = service.current().engine.node_registry().clone();
    let (stats, records) = registry.connection_records(true);
    assert_eq!(stats.started, 4);
    assert_eq!(records.len(), 4);
    for flow in &records {
        assert_eq!(flow.context.origin, FlowOrigin::Gateway);
        assert!(flow.context.process.is_none());
        assert_eq!(flow.routing.configuration_revision, 1);
        assert!(flow.context.source_ip.is_some_and(|ip| {
            [
                "10.231.83.2".parse::<IpAddr>().unwrap(),
                "fd42:6870:7300:1::2".parse().unwrap(),
            ]
            .contains(&ip)
        }));
        assert!(flow.routing.rule_id.is_some());
        assert!(flow.choice.is_some());
    }
    assert_eq!(socks.associations.load(Ordering::Relaxed), 2);
    assert_eq!(socks.datagrams.load(Ordering::Relaxed), 2);
    assert_eq!(upstream.node_registry().connection_statistics().started, 2);
    let evidence = service.linux_capture_status().await;
    assert!(evidence.ready);
    set_up(_veth.0, false);
    suspended(&service, true).await;
    set_up(_veth.0, true);
    recheck(&service).await;
    rename(_veth.0, "hpslan-renamed");
    suspended(&service, false).await;
    rename(_veth.0, "hpslan0");
    recheck(&service).await;
    peer_up(&namespace, false);
    suspended(&service, true).await;
    peer_up(&namespace, true);
    recheck(&service).await;
    service.shutdown().await.unwrap();
    upstream.shutdown().await;
    socks.shutdown().await;
    println!(
        "{}",
        serde_json::json!({"scope":"actual_gateway_shared_engine_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":evidence.object_fingerprint,"ipv4_ipv6_tcp_udp":true,"tcp_http_upstream_flows":2,"udp_socks5_associations":2,"source_rules_and_fixed_groups":true,"gateway_process_unknown":true,"interface_down_suspended":true,"carrier_loss_suspended_with_links_retained":true,"interface_rename_suspended":true,"interface_restore_recheck_ready":true,"bypassed_tcp_flows":2,"tcp_payload_bytes_per_flow_each_direction":262144,"udp_payload_bytes_per_flow_each_direction":1200,"full_transparent_datapath_ready":false})
    );
}
