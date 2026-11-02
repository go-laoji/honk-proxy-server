use super::test_support::{Address, Group, Jobs};
use super::{CaptureSockets, PreparedCapture};
use hps_config::{
    GroupConfig, GroupStrategy, InboundConfig, LinuxCaptureConfig, PortRange, ProcessCondition,
    ProxyConfig, ProxyProtocol, RuleConditions, RuleConfig, ServerTls, UpstreamConfig,
};
use hps_domain::{FlowOrigin, RouteAction};
use hps_engine::runtime::Engine;
use std::{
    io::{Read, Write},
    net::SocketAddr,
    os::fd::AsFd,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[test]
#[ignore = "helper invoked only by the isolated real_transparent_tcp_shared_engine test"]
fn transparent_tcp_child() {
    let group = std::env::var("HPS_TRANSPARENT_TCP_CGROUP").unwrap();
    let group = Path::new(&group).canonicalize().unwrap();
    assert!(
        group
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hps-transparent-tcp-")
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(group.join("cgroup.procs"))
        .unwrap()
        .write_all(format!("{}\n", std::process::id()).as_bytes())
        .unwrap();
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_NAME, c"hps-tp-client".as_ptr(), 0, 0, 0) },
        0
    );
    let targets: Vec<SocketAddr> =
        serde_json::from_str(&std::env::var("HPS_TRANSPARENT_TCP_TARGETS").unwrap()).unwrap();
    assert_eq!(targets.len(), 6);
    for target in targets {
        let mut stream =
            std::net::TcpStream::connect_timeout(&target, Duration::from_secs(3)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        assert_eq!(stream.peer_addr().unwrap(), target);
        stream.write_all(b"shared-engine-tcp").unwrap();
        let mut reply = [0; 19];
        stream.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"shared-engine-reply");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires an isolated privileged Linux namespace, writable cgroup v2 and actual eBPF object; assigns only EXCL-owned fixture addresses"]
async fn real_transparent_tcp_shared_engine() {
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
    let group = Group(Path::new(&root).join(format!("hps-transparent-tcp-{}", std::process::id())));
    std::fs::create_dir(&group.0).unwrap();
    let _addresses = [Address::add("203.0.113.77"), Address::add("2001:db8::77")];
    let directory = tempfile::tempdir().unwrap();
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let tls = ServerTls {
        certificate: directory.path().join("certificate.pem"),
        private_key: directory.path().join("key.pem"),
    };
    std::fs::write(&tls.certificate, certificate.cert.pem()).unwrap();
    std::fs::write(&tls.private_key, certificate.signing_key.serialize_pem()).unwrap();
    let protocols = [
        ProxyProtocol::Http,
        ProxyProtocol::Https,
        ProxyProtocol::Socks5,
    ];
    let ids = ["http", "https", "socks5"];
    let mut upstream_config = ProxyConfig::default();
    upstream_config.routing.default_action = RouteAction::Direct;
    for (id, protocol) in ids.into_iter().zip(protocols) {
        upstream_config.inbounds.push(InboundConfig {
            id: id.into(),
            protocol,
            listen: "127.0.0.1:0".parse().unwrap(),
            enabled: true,
            tls: (protocol == ProxyProtocol::Https).then_some(tls.clone()),
            max_connections: 16,
            bytes_per_second: 0,
        });
    }
    let upstream = Engine::new(upstream_config).unwrap();
    upstream.set_generation(1);
    upstream.start_enabled().await.unwrap();
    let mut config = ProxyConfig::default();
    config.routing.default_action = RouteAction::Reject;
    let mut jobs = Jobs(vec![]);
    let mut targets = vec![];
    for (id, protocol) in ids.into_iter().zip(protocols) {
        let entry = upstream
            .status()
            .inbounds
            .into_iter()
            .find(|e| e.id == id)
            .unwrap();
        let address: SocketAddr = entry.listen.unwrap().parse().unwrap();
        config.upstreams.push(UpstreamConfig {
            id: id.into(),
            protocol,
            host: address.ip().to_string(),
            port: address.port(),
            enabled: true,
            credentials: None,
            expires_at_unix: None,
            ca_certificate: (protocol == ProxyProtocol::Https).then_some(tls.certificate.clone()),
            tls_server_name: (protocol == ProxyProtocol::Https).then_some("localhost".into()),
        });
        config.selection.groups.push(GroupConfig {
            id: id.into(),
            strategy: GroupStrategy::Fixed,
            members: vec![id.into()],
            pinned_node: Some(id.into()),
            ..Default::default()
        });
        let mut ports = vec![];
        let mut chosen_port = 0;
        for ip in ["203.0.113.77", "2001:db8::77"] {
            let listener = TcpListener::bind(SocketAddr::new(ip.parse().unwrap(), chosen_port))
                .await
                .unwrap();
            let target = listener.local_addr().unwrap();
            targets.push(target);
            if chosen_port == 0 {
                ports.push(PortRange {
                    start: target.port(),
                    end: target.port(),
                });
                chosen_port = target.port();
            }
            jobs.0.push(tokio::spawn(async move {
                let (mut origin, _) = listener.accept().await.unwrap();
                let mut body = [0; 17];
                origin.read_exact(&mut body).await.unwrap();
                assert_eq!(&body, b"shared-engine-tcp");
                origin.write_all(b"shared-engine-reply").await.unwrap();
            }));
        }
        config.routing.rules.push(RuleConfig {
            id: id.into(),
            enabled: true,
            conditions: RuleConditions {
                destination_ports: ports,
                process: Some(ProcessCondition {
                    command: Some("hps-tp-client".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            action: RouteAction::Group { id: id.into() },
        });
    }
    let specification = LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        udp: false,
        tcp_port: 0,
        ..Default::default()
    };
    let tcp4 = hps_net::transparent::bind_tcp(
        "0.0.0.0:0".parse().unwrap(),
        specification.bypass_mark,
        128,
    )
    .unwrap();
    let tcp6 =
        hps_net::transparent::bind_tcp("[::]:0".parse().unwrap(), specification.bypass_mark, 128)
            .unwrap();
    let capture = Arc::new(
        PreparedCapture::read(&specification)
            .unwrap()
            .load(CaptureSockets {
                tcp4: Some(tcp4.as_fd()),
                tcp6: Some(tcp6.as_fd()),
                udp4: None,
                udp6: None,
            })
            .unwrap(),
    );
    capture.prepare_routes().unwrap();
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
    let pids = Arc::new(Mutex::new(vec![]));
    let mut accept_jobs = vec![];
    for listener in [tcp4, tcp6] {
        let capture = capture.clone();
        let engine = engine.clone();
        let pids = pids.clone();
        accept_jobs.push(tokio::spawn(async move {
            for _ in 0..3 {
                let (stream, source) = listener.accept().await.unwrap();
                let destination = stream.local_addr().unwrap();
                let (revision, context) = capture.routing_context(source, destination, 6).unwrap();
                assert_eq!(context.origin, FlowOrigin::LinuxLocal);
                let process = context.process.as_ref().unwrap();
                assert_eq!(process.command, "hps-tp-client");
                pids.lock().unwrap().push(process.pid);
                engine
                    .spawn_captured_tcp(stream, context, revision)
                    .unwrap();
            }
        }));
    }
    capture.activate(7, true).unwrap();
    let exe = std::env::current_exe().unwrap();
    let scope = group.0.clone();
    let targets = serde_json::to_string(&targets).unwrap();
    let (pid, output) = tokio::task::spawn_blocking(move || {
        let child = Command::new(exe)
            .args([
                "--exact",
                "transparent::tcp_tests::transparent_tcp_child",
                "--ignored",
                "--nocapture",
            ])
            .env("HPS_TRANSPARENT_TCP_CGROUP", scope)
            .env("HPS_TRANSPARENT_TCP_TARGETS", targets)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        (child.id(), child.wait_with_output().unwrap())
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for job in accept_jobs {
        tokio::time::timeout(Duration::from_secs(2), job)
            .await
            .unwrap()
            .unwrap();
    }
    for job in &mut jobs.0 {
        tokio::time::timeout(Duration::from_secs(2), job)
            .await
            .unwrap()
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while !engine.is_idle() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pids.lock().unwrap().as_slice(), &[pid; 6]);
    let (stats, flows) = engine.node_registry().connection_records(true);
    assert_eq!(stats.opened, 6);
    assert_eq!(stats.failed, 0);
    assert_eq!(stats.bytes_up, 6 * 17);
    assert_eq!(stats.bytes_down, 6 * 19);
    for id in ids {
        let selected = flows
            .iter()
            .filter(|f| f.choice.as_ref().is_some_and(|c| c.node_id == id))
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 2);
        assert!(
            selected
                .iter()
                .all(|f| f.routing.configuration_revision == 7
                    && f.routing.rule_id.as_deref() == Some(id)
                    && f.selection.as_ref().unwrap().group.as_deref() == Some(id)
                    && f.context.process.as_ref().unwrap().pid == pid)
        );
    }
    let (upstream_stats, upstream_flows) = upstream.node_registry().connection_records(true);
    assert_eq!(upstream_stats.opened, 6);
    assert!(
        upstream_flows
            .iter()
            .all(|f| f.context.origin == FlowOrigin::Explicit
                && f.context.process.is_none()
                && f.routing.action == RouteAction::Direct)
    );
    assert!(
        capture
            .stats()
            .unwrap()
            .iter()
            .all(|s| s.errors == 0 && s.missing_process == 0)
    );
    capture.shutdown().unwrap();
    engine.shutdown().await;
    upstream.shutdown().await;
    println!(
        "{}",
        serde_json::json!({"scope":"local_tcp_shared_engine_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":capture.fingerprint,"revision":7,"verified_flows":6,"client_pid":pid,"ipv4":true,"ipv6":true,"http_upstream":true,"https_upstream":true,"socks5_upstream":true,"process_rules_and_fixed_groups":true,"business_bytes_up":stats.bytes_up,"business_bytes_down":stats.bytes_down,"forwarding_verified":true,"full_transparent_datapath_ready":false})
    );
}
