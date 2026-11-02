#![cfg(target_os = "linux")]
#[path = "../../platform/src/transparent/test_support.rs"]
#[allow(
    dead_code,
    reason = "shared fixture includes unrelated cgroup membership helpers"
)]
mod capture_support;
#[path = "support/network.rs"]
#[allow(
    dead_code,
    reason = "shared network fixture includes interface fault helpers"
)]
mod network;
#[path = "../../platform/src/rtnetlink.rs"]
#[allow(
    dead_code,
    reason = "shared bounded RTNL fixture includes unused inspection helpers"
)]
mod rtnetlink;
#[path = "../../platform/src/transparent/test_proxy.rs"]
mod test_proxy;
use capture_support::{Address, Group};
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{
    DaemonConfig, GroupConfig, GroupStrategy, InboundConfig, LinuxCaptureConfig, ProcessCondition,
    ProxyConfig, ProxyProtocol, RuleConditions, RuleConfig, UpstreamConfig,
};
use hps_domain::{FlowOrigin, RouteAction, SelectionTransport};
use network::{Veth, address, default_route, index, request, up};
use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::Ordering,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct OwnedChild(std::process::Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Route(Vec<u8>);
impl Route {
    fn add(ip: IpAddr, gateway: IpAddr) -> Self {
        let (family, prefix, dest, via) = match (ip, gateway) {
            (IpAddr::V4(ip), IpAddr::V4(gw)) => (
                libc::AF_INET,
                32,
                ip.octets().to_vec(),
                gw.octets().to_vec(),
            ),
            (IpAddr::V6(ip), IpAddr::V6(gw)) => (
                libc::AF_INET6,
                128,
                ip.octets().to_vec(),
                gw.octets().to_vec(),
            ),
            _ => panic!("fixture family mismatch"),
        };
        let mut payload = vec![family as u8, prefix, 0, 0, 254, 4, 0, 1, 0, 0, 0, 0];
        rtnetlink::attribute(&mut payload, 1, &dest);
        rtnetlink::attribute(&mut payload, 5, &via);
        rtnetlink::attribute(&mut payload, 4, &index("hpslan0").to_ne_bytes());
        request(libc::RTM_NEWROUTE, &payload, true);
        Self(payload)
    }
}
impl Drop for Route {
    fn drop(&mut self) {
        let _ = rtnetlink::exchange(
            libc::RTM_DELROUTE,
            (libc::NLM_F_REQUEST | libc::NLM_F_ACK) as u16,
            &self.0,
            false,
        );
    }
}
fn assert_targets_are_not_local() {
    for reply in rtnetlink::exchange(
        libc::RTM_GETADDR,
        (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16,
        &[0; 8],
        true,
    )
    .unwrap()
    {
        if reply.kind != libc::RTM_NEWADDR || reply.payload.len() < 8 {
            continue;
        }
        let attrs = rtnetlink::attributes(&reply.payload, 8).unwrap();
        for bytes in attrs.values() {
            assert_ne!(bytes, &[203, 0, 113, 77]);
            assert_ne!(
                bytes,
                &"2001:db8::77"
                    .parse::<std::net::Ipv6Addr>()
                    .unwrap()
                    .octets()
            );
        }
    }
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "helper invoked only by real_nonlocal_destination_policy_route"]
async fn nonlocal_origin() {
    assert_eq!(unsafe { libc::unshare(libc::CLONE_NEWNET) }, 0);
    let mut control =
        tokio::net::UnixStream::connect(std::env::var("HPS_NONLOCAL_CONTROL").unwrap())
            .await
            .unwrap();
    control
        .write_u32_le(unsafe { libc::syscall(libc::SYS_gettid) } as u32)
        .await
        .unwrap();
    let mut signal = [0];
    control.read_exact(&mut signal).await.unwrap();
    assert_eq!(signal, [b'P']);
    up("lo");
    up("hpsclient0");
    address("hpsclient0", "10.231.83.2");
    address("hpsclient0", "fd42:6870:7300:1::2");
    default_route("10.231.83.1");
    default_route("fd42:6870:7300:1::1");
    let _addresses = [Address::add("203.0.113.77"), Address::add("2001:db8::77")];
    let mut targets = vec![];
    let mut jobs = vec![];
    for ip in ["203.0.113.77", "2001:db8::77"] {
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(ip.parse().unwrap(), 0))
            .await
            .unwrap();
        targets.push(listener.local_addr().unwrap());
        jobs.push(tokio::spawn(async move {
            let (mut stream, peer) = listener.accept().await.unwrap();
            assert_eq!(
                peer.ip().to_string(),
                if peer.is_ipv4() {
                    "10.231.83.1"
                } else {
                    "fd42:6870:7300:1::1"
                }
            );
            let mut data = [0; 4096];
            stream.read_exact(&mut data).await.unwrap();
            assert_eq!(data, [b'T'; 4096]);
            stream.write_all(&data).await.unwrap();
        }));
    }
    for ip in ["203.0.113.77", "2001:db8::77"] {
        let socket = tokio::net::UdpSocket::bind(SocketAddr::new(ip.parse().unwrap(), 0))
            .await
            .unwrap();
        targets.push(socket.local_addr().unwrap());
        jobs.push(tokio::spawn(async move {
            let mut data = [0; 1024];
            let (n, peer) = socket.recv_from(&mut data).await.unwrap();
            assert_eq!(
                peer.ip().to_string(),
                if peer.is_ipv4() {
                    "10.231.83.1"
                } else {
                    "fd42:6870:7300:1::1"
                }
            );
            assert_eq!(&data[..n], &[b'U'; 512]);
            socket.send_to(&data[..n], peer).await.unwrap();
        }));
    }
    let bytes = serde_json::to_vec(&targets).unwrap();
    control.write_u32_le(bytes.len() as u32).await.unwrap();
    control.write_all(&bytes).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        for job in jobs {
            job.await.unwrap();
        }
    })
    .await
    .unwrap();
}
#[test]
#[ignore = "helper invoked only by real_nonlocal_destination_policy_route"]
fn nonlocal_client() {
    let group = std::env::var("HPS_NONLOCAL_GROUP").unwrap();
    assert!(
        Path::new(&group)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hps-transparent-nonlocal-")
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(Path::new(&group).join("cgroup.procs"))
        .unwrap()
        .write_all(format!("{}\n", std::process::id()).as_bytes())
        .unwrap();
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_NAME, c"hps-nonlocal".as_ptr(), 0, 0, 0) },
        0
    );
    let targets: Vec<SocketAddr> =
        serde_json::from_str(&std::env::var("HPS_NONLOCAL_TARGETS").unwrap()).unwrap();
    for target in &targets[..2] {
        let mut stream =
            std::net::TcpStream::connect_timeout(target, Duration::from_secs(5)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(&[b'T'; 4096]).unwrap();
        let mut data = [0; 4096];
        stream.read_exact(&mut data).unwrap();
        assert_eq!(data, [b'T'; 4096]);
    }
    for target in &targets[2..] {
        let socket = std::net::UdpSocket::bind(if target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket.send_to(&[b'U'; 512], target).unwrap();
        let mut data = [0; 1024];
        let (n, source) = socket.recv_from(&mut data).unwrap();
        assert_eq!(source, *target);
        assert_eq!(&data[..n], &[b'U'; 512]);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux, child target netns, actual routes and eBPF"]
async fn real_nonlocal_destination_policy_route() {
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
    let group =
        Group(Path::new(&root).join(format!("hps-transparent-nonlocal-{}", std::process::id())));
    std::fs::create_dir(&group.0).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("origin.sock");
    let control = tokio::net::UnixListener::bind(&path).unwrap();
    let mut origin = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "nonlocal_origin", "--ignored", "--nocapture"])
            .env("HPS_NONLOCAL_CONTROL", &path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let (mut control, _) = tokio::time::timeout(Duration::from_secs(5), control.accept())
        .await
        .unwrap()
        .unwrap();
    let tid = control.read_u32_le().await.unwrap();
    let namespace =
        std::fs::File::open(format!("/proc/{}/task/{tid}/ns/net", origin.0.id())).unwrap();
    let _veth = Veth::create(&namespace);
    control.write_all(b"P").await.unwrap();
    let length = control.read_u32_le().await.unwrap();
    assert!(length < 4096);
    let mut bytes = vec![0; length as usize];
    control.read_exact(&mut bytes).await.unwrap();
    let targets: Vec<SocketAddr> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(targets.len(), 4);
    assert_targets_are_not_local();
    let _routes = [
        Route::add(
            "203.0.113.77".parse().unwrap(),
            "10.231.83.2".parse().unwrap(),
        ),
        Route::add(
            "2001:db8::77".parse().unwrap(),
            "fd42:6870:7300:1::2".parse().unwrap(),
        ),
    ];
    let mut proxy = ProxyConfig::default();
    proxy.routing.default_action = RouteAction::Direct;
    proxy.inbounds.push(InboundConfig {
        id: "nonlocal-http".into(),
        protocol: ProxyProtocol::Http,
        listen: "127.0.0.1:0".parse().unwrap(),
        enabled: true,
        tls: None,
        max_connections: 16,
        bytes_per_second: 0,
    });
    let http = hps_engine::runtime::Engine::new(proxy).unwrap();
    http.set_generation(1);
    http.start_enabled().await.unwrap();
    let endpoint: SocketAddr = http.status().inbounds[0]
        .listen
        .as_ref()
        .unwrap()
        .parse()
        .unwrap();
    let mut socks = test_proxy::Proxy::bind().await;
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.dns.enabled = false;
    config.proxy.routing.default_action = RouteAction::Reject;
    for (id, protocol, endpoint, transport) in [
        (
            "http",
            ProxyProtocol::Http,
            endpoint,
            SelectionTransport::Tcp,
        ),
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
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
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
                process: Some(ProcessCondition {
                    command: Some("hps-nonlocal".into()),
                    ..Default::default()
                }),
                transports: vec![transport],
                ..Default::default()
            },
            action: RouteAction::Group { id: id.into() },
        });
    }
    config.linux_capture = Some(LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        tcp_port: 0,
        udp_port: 0,
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
    let client = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "nonlocal_client", "--ignored", "--nocapture"])
        .env("HPS_NONLOCAL_GROUP", &group.0)
        .env(
            "HPS_NONLOCAL_TARGETS",
            serde_json::to_string(&targets).unwrap(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = client.id();
    let output = tokio::task::spawn_blocking(move || client.wait_with_output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}\nrecords={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        service
            .current()
            .engine
            .node_registry()
            .connection_records(true)
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(status) = origin.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let (stats, flows) = service
        .current()
        .engine
        .node_registry()
        .connection_records(true);
    assert_eq!(stats.started, 4);
    assert_eq!(flows.len(), 4);
    assert_eq!((stats.bytes_up, stats.bytes_down), (9216, 9216));
    for flow in flows {
        assert_eq!(flow.context.origin, FlowOrigin::LinuxLocal);
        assert_eq!(flow.context.process.unwrap().pid, pid);
        assert!(flow.context.source_ip.unwrap().is_loopback());
        assert!(flow.choice.is_some());
        assert_eq!(flow.routing.configuration_revision, 1);
    }
    assert_eq!(http.node_registry().connection_statistics().started, 2);
    assert_eq!(socks.associations.load(Ordering::Relaxed), 2);
    assert_eq!(socks.datagrams.load(Ordering::Relaxed), 2);
    let capture = service.linux_capture_status().await;
    service.shutdown().await.unwrap();
    http.shutdown().await;
    socks.shutdown().await;
    println!(
        "{}",
        serde_json::json!({"scope":"nonlocal_target_policy_route_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":capture.object_fingerprint,"targets_not_assigned_in_proxy_namespace":true,"actual_target_in_separate_network_namespace":true,"unmarked_daemon_egress_through_veth":true,"ipv4_ipv6_tcp_udp":true,"captured_process_verified":true,"http_upstream_flows":2,"socks5_udp_associations":2,"bytes_each_direction":9216,"original_reply_source":true,"full_transparent_datapath_ready":false})
    );
}
