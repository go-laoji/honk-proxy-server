#![cfg(target_os = "linux")]
#[path = "../../platform/src/transparent/test_support.rs"]
mod capture_support;
#[path = "../../platform/src/rtnetlink.rs"]
#[allow(
    dead_code,
    reason = "shared route fixture also supports decoded response inspection in platform tests"
)]
mod rtnetlink;
use capture_support::{Address, Group, Jobs, Membership};
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{DaemonConfig, LinuxCaptureConfig, ProcessCondition, RuleConditions, RuleConfig};
use hps_domain::{ConfigActivate, ConfigInput, FlowOrigin, RouteAction};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
#[ignore = "helper invoked only by the isolated real_transparent_supervisor test"]
fn transparent_supervisor_child() {
    let group = std::path::PathBuf::from(std::env::var("HPS_DAEMON_CAPTURE_CGROUP").unwrap())
        .canonicalize()
        .unwrap();
    assert!(
        group
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hps-transparent-supervisor-")
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(group.join("cgroup.procs"))
        .unwrap()
        .write_all(format!("{}\n", std::process::id()).as_bytes())
        .unwrap();
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_NAME, c"hpsd-client".as_ptr(), 0, 0, 0) },
        0
    );
    let targets: Vec<SocketAddr> =
        serde_json::from_str(&std::env::var("HPS_DAEMON_CAPTURE_TARGETS").unwrap()).unwrap();
    assert_eq!(targets.len(), 4);
    let mut control = std::os::unix::net::UnixStream::connect(
        std::env::var("HPS_DAEMON_CAPTURE_CONTROL").unwrap(),
    )
    .unwrap();
    control
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut tcp = targets[..2]
        .iter()
        .map(|target| {
            let s = std::net::TcpStream::connect_timeout(target, Duration::from_secs(5)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            s.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            s
        })
        .collect::<Vec<_>>();
    let udp = targets[2..]
        .iter()
        .map(|target| {
            let s = std::net::UdpSocket::bind(if target.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            })
            .unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            (s, *target)
        })
        .collect::<Vec<_>>();
    let freeze = std::env::var("HPS_DAEMON_CAPTURE_FREEZE").is_ok();
    let phases = if freeze { 2 } else { 3 };
    for phase in 0..phases {
        for stream in &mut tcp {
            let body = [b't', phase];
            stream.write_all(&body).unwrap();
            let mut reply = [0; 2];
            stream.read_exact(&mut reply).unwrap();
            assert_eq!(reply, body);
        }
        for (socket, target) in &udp {
            let body = [b'u', phase];
            socket.send_to(&body, target).unwrap();
            let mut reply = [0; 64];
            let (n, source) = socket.recv_from(&mut reply).unwrap();
            assert_eq!((n, source), (2, *target));
            assert_eq!(&reply[..n], &body);
        }
        if std::env::var("HPS_DAEMON_CAPTURE_DNS").is_ok() {
            captured_dns(phase);
        }
        if phase > 0 && !freeze {
            for target in &targets[..2] {
                let mut s =
                    std::net::TcpStream::connect_timeout(target, Duration::from_secs(5)).unwrap();
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let _ = s.write_all(b"rejected-new");
                let mut reply = [0; 1];
                match s.read(&mut reply) {
                    Ok(0) => {}
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) => {}
                    other => panic!("new TCP flow was not rejected: {other:?}"),
                };
            }
            for target in &targets[2..] {
                let s = std::net::UdpSocket::bind(if target.is_ipv4() {
                    "0.0.0.0:0"
                } else {
                    "[::]:0"
                })
                .unwrap();
                s.set_read_timeout(Some(Duration::from_millis(300)))
                    .unwrap();
                s.send_to(b"rejected-new", target).unwrap();
                let mut reply = [0; 64];
                let error = s.recv_from(&mut reply).unwrap_err();
                assert!(matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ));
            }
        }
        control.write_all(&[b'0' + phase]).unwrap();
        if phase + 1 < phases {
            let mut next = [0];
            control.read_exact(&mut next).unwrap();
            assert_eq!(next, [b'N']);
        }
    }
}
fn ids() -> BTreeSet<u32> {
    aya::programs::loaded_programs()
        .filter_map(Result::ok)
        .map(|p| p.id())
        .collect()
}
fn input(config: &DaemonConfig, expected: i64) -> ConfigInput {
    ConfigInput {
        toml: toml::to_string_pretty(config).unwrap(),
        expected_revision: expected,
        expected_draft_version: None,
        note: "transparent supervisor validation".into(),
        use_draft_credentials: false,
    }
}
async fn apply(service: &std::sync::Arc<Supervisor>, config: &DaemonConfig, expected: i64) -> i64 {
    let revision = service
        .create_revision(input(config, expected))
        .await
        .unwrap();
    let result = service
        .activate(ConfigActivate {
            revision: revision.id,
            expected_revision: expected,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    assert_eq!(result.state, "applied");
    assert_eq!(result.running_revision, revision.id);
    revision.id
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux, writable cgroup v2 and actual eBPF object"]
async fn real_transparent_supervisor() {
    run_supervisor(false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux, cgroup v2 and freezing an actual owned BPF policy map"]
async fn real_transparent_publication_failure_restores_deferred_head() {
    run_supervisor(true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux and actual DNS plus transparent eBPF programs"]
async fn real_transparent_dns_coexist_and_publication_rollback() {
    run_supervisor(true, true).await;
}
async fn run_supervisor(freeze: bool, dns: bool) {
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
    let group =
        Group(Path::new(&root).join(format!("hps-transparent-supervisor-{}", std::process::id())));
    std::fs::create_dir(&group.0).unwrap();
    let _addresses = [Address::add("203.0.113.77"), Address::add("2001:db8::77")];
    let directory = tempfile::tempdir().unwrap();
    let control_path = directory.path().join("control.sock");
    let control = tokio::net::UnixListener::bind(&control_path).unwrap();
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.dns.enabled = false;
    config.proxy.routing.default_action = RouteAction::Reject;
    config.proxy.routing.rules = vec![RuleConfig {
        id: "captured-process".into(),
        enabled: true,
        conditions: RuleConditions {
            process: Some(ProcessCondition {
                command: Some("hpsd-client".into()),
                ..Default::default()
            }),
            inbound_protocols: vec!["transparent".into()],
            ..Default::default()
        },
        action: RouteAction::Direct,
    }];
    config.linux_capture = Some(LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        tcp_port: 0,
        udp_port: 0,
        max_flows: 64,
        max_pending: 16,
        max_udp_sessions: 16,
        udp_idle_secs: 30,
        ..Default::default()
    });
    let mut jobs = Jobs(vec![]);
    let mut targets = vec![];
    for ip in ["203.0.113.77", "2001:db8::77"] {
        let listener = tokio::net::TcpListener::bind(SocketAddr::new(ip.parse().unwrap(), 0))
            .await
            .unwrap();
        targets.push(listener.local_addr().unwrap());
        jobs.0.push(tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 2];
            for _ in 0..(if freeze { 2 } else { 3 }) {
                stream.read_exact(&mut bytes).await.unwrap();
                stream.write_all(&bytes).await.unwrap();
            }
        }));
    }
    for ip in ["203.0.113.77", "2001:db8::77"] {
        let socket = hps_net::transparent::bind_udp(
            SocketAddr::new(ip.parse().unwrap(), 0),
            config.linux_capture.as_ref().unwrap().bypass_mark,
        )
        .unwrap();
        targets.push(socket.local_addr().unwrap());
        jobs.0.push(tokio::spawn(async move {
            let mut bytes = [0; 64];
            for _ in 0..(if freeze { 2 } else { 3 }) {
                let (n, peer) = socket.recv_from(&mut bytes).await.unwrap();
                socket.send_to(&bytes[..n], peer).await.unwrap();
            }
        }));
    }
    let mut dns_jobs = Jobs(vec![]);
    let old_dns_requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let new_dns_requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut replacement_dns = None;
    if dns {
        let old = dns_upstream(9, old_dns_requests.clone(), &mut dns_jobs).await;
        let new = dns_upstream(8, new_dns_requests.clone(), &mut dns_jobs).await;
        replacement_dns = Some(new);
        let port = loop {
            let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = reservation.local_addr().unwrap().port();
            if port != old.port() && port != new.port() && !targets.iter().any(|t| t.port() == port)
            {
                break port;
            }
        };
        config.dns.enabled = true;
        config.dns.upstreams[0].protocol = hps_config::DnsProtocol::Udp;
        config.dns.upstreams[0].addresses = vec![old];
        config.dns.capture = Some(hps_config::DnsCaptureConfig {
            object: config.linux_capture.as_ref().unwrap().object.clone(),
            cgroup: group.0.clone(),
            port,
        });
    }
    let before = ids();
    let store = hps_storage::Store::open(Some(directory.path()), config.clone()).unwrap();
    let service = Supervisor::boot(store, StartupOverrides::default())
        .await
        .unwrap();
    service.confirm_startup().await.unwrap();
    let membership = Membership::enter(&group.0);
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "transparent_supervisor_child",
            "--ignored",
            "--nocapture",
        ])
        .envs(freeze.then_some(("HPS_DAEMON_CAPTURE_FREEZE", "1")))
        .envs(dns.then_some(("HPS_DAEMON_CAPTURE_DNS", "1")))
        .env("HPS_DAEMON_CAPTURE_CGROUP", &group.0)
        .env(
            "HPS_DAEMON_CAPTURE_TARGETS",
            serde_json::to_string(&targets).unwrap(),
        )
        .env("HPS_DAEMON_CAPTURE_CONTROL", &control_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let pid = child.id();
    let (mut control, _) = tokio::time::timeout(Duration::from_secs(5), control.accept())
        .await
        .unwrap()
        .unwrap();
    let mut stage = [0];
    tokio::time::timeout(Duration::from_secs(10), control.read_exact(&mut stage))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stage, [b'0']);
    let (_, flows) = service
        .current()
        .engine
        .node_registry()
        .connection_records(false);
    assert_eq!(flows.len(), 4);
    assert!(
        flows.iter().all(|f| f.routing.configuration_revision == 1
            && f.context.process.as_ref().unwrap().pid == pid)
    );
    let boot_programs = ids().difference(&before).copied().collect::<BTreeSet<_>>();
    assert_eq!(boot_programs.len(), if dns { 16 } else { 8 });
    let expected_revision = if freeze {
        config.instance_name = "deferred-before-failure".into();
        let deferred = service.create_revision(input(&config, 1)).await.unwrap();
        let pending = service
            .activate(ConfigActivate {
                revision: deferred.id,
                expected_revision: 1,
                defer_until_restart: true,
            })
            .await
            .unwrap();
        assert_eq!(pending.state, "pending_restart");
        assert_eq!(pending.running_revision, 1);
        let policy_ids = aya::programs::loaded_programs()
            .filter_map(Result::ok)
            .filter(|p| boot_programs.contains(&p.id()))
            .flat_map(|p| p.map_ids().unwrap().unwrap_or_default())
            .collect::<BTreeSet<_>>();
        let policy = policy_ids
            .into_iter()
            .find(|id| {
                aya::maps::MapInfo::from_id(*id).is_ok_and(|info| info.name() == b"HPS_TP_POLICY")
            })
            .unwrap();
        let map = aya::maps::MapData::from_id(policy).unwrap();
        use std::os::fd::{AsFd, AsRawFd};
        let fd = map.fd().as_fd().as_raw_fd() as u32;
        assert_eq!(
            unsafe { libc::syscall(libc::SYS_bpf, 22, &fd, std::mem::size_of::<u32>()) },
            0
        );
        drop(map);
        config.proxy.routing.rules[0].action = RouteAction::Reject;
        if let Some(address) = replacement_dns {
            config.dns.upstreams[0].addresses = vec![address];
        }
        let failed = service
            .create_revision(input(&config, deferred.id))
            .await
            .unwrap();
        assert!(matches!(
            service
                .activate(ConfigActivate {
                    revision: failed.id,
                    expected_revision: deferred.id,
                    defer_until_restart: false
                })
                .await,
            Err(hps_application::ApplicationError::Capture(_))
        ));
        let workspace = service.workspace().await.unwrap();
        assert_eq!(
            (workspace.running_revision, workspace.desired_revision),
            (1, deferred.id)
        );
        assert!(
            workspace
                .pending_restart
                .iter()
                .any(|reason| reason == "explicit_restart_requested")
        );
        assert!(workspace.activations.iter().any(|a| a.revision == failed.id
            && a.state == "failed"
            && a.error_code.as_deref() == Some("transparent_publication_failed")));
        assert!(
            !workspace
                .activations
                .iter()
                .any(|a| a.revision == failed.id && a.state == "prepared")
        );
        control.write_all(b"N").await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), control.read_exact(&mut stage))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stage, [b'1']);
        deferred.id
    } else {
        config.proxy.routing.rules[0].action = RouteAction::Reject;
        let second = apply(&service, &config, 1).await;
        control.write_all(b"N").await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), control.read_exact(&mut stage))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stage, [b'1']);
        config.instance_name = "capture-name-only-revision".into();
        let third = apply(&service, &config, second).await;
        assert_eq!(service.current().engine.generation(), third);
        control.write_all(b"N").await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), control.read_exact(&mut stage))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stage, [b'2']);
        third
    };
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for task in &mut jobs.0 {
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap();
    }
    if dns {
        assert_eq!(
            old_dns_requests.load(std::sync::atomic::Ordering::Relaxed),
            8
        );
        assert_eq!(
            new_dns_requests.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        assert!(service.dns_overview(None, 50).unwrap().linux_capture_active);
    }
    let registry = service.current().engine.node_registry();
    let (_, flows) = registry.connection_records(true);
    let old = flows
        .iter()
        .filter(|f| f.routing.configuration_revision == 1)
        .collect::<Vec<_>>();
    assert_eq!(old.len(), 4);
    assert!(
        old.iter()
            .all(|f| f.context.origin == FlowOrigin::LinuxLocal
                && f.context.process.as_ref().unwrap().pid == pid
                && f.routing.rule_id.as_deref() == Some("captured-process")
                && f.bytes_up == (if freeze { 4 } else { 6 })
                && f.bytes_down == (if freeze { 4 } else { 6 }))
    );
    for revision in if freeze { vec![] } else { vec![2, 3] } {
        let rejected = flows
            .iter()
            .filter(|f| f.routing.configuration_revision == revision)
            .collect::<Vec<_>>();
        assert_eq!(rejected.len(), 4);
        assert!(
            rejected
                .iter()
                .all(|f| f.state == "rejected" && f.bytes_up == 0 && f.bytes_down == 0)
        );
    }
    for flow in flows.iter().filter(|f| f.state == "open") {
        registry.disconnect_connection(&flow.id);
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if service.current().engine.is_idle()
                && service.workspace().await.unwrap().draining_generations == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let owned = ids().difference(&before).copied().collect::<BTreeSet<_>>();
    assert_eq!(owned.len(), if dns { 16 } else { 8 });
    let shutdown = service.shutdown().await;
    if freeze {
        assert!(matches!(
            shutdown,
            Err(hps_application::ApplicationError::Capture(_))
        ));
    } else {
        shutdown.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while owned.iter().any(|id| ids().contains(id)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(membership);
    let restarted = Supervisor::boot(
        hps_storage::Store::open(Some(directory.path()), DaemonConfig::default()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    restarted.confirm_startup().await.unwrap();
    assert_eq!(
        restarted.workspace().await.unwrap().running_revision,
        expected_revision
    );
    assert_eq!(restarted.current().engine.generation(), expected_revision);
    restarted.shutdown().await.unwrap();
    println!(
        "{}",
        serde_json::json!({"scope":if dns{"daemon_dns_transparent_rollback"}else if freeze{"daemon_transparent_publication_failure"}else{"daemon_transparent_local_only"},"kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"client_pid":pid,"ipv4":true,"ipv6":true,"tcp":true,"udp":true,"actual_process_rule":true,"old_revision":1,"final_desired_revision":expected_revision,"old_connections_retained":4,"new_connections_rejected":if freeze{0}else{8},"business_bytes_up":if freeze{16}else{24},"business_bytes_down":if freeze{16}else{24},"dns_tcp_udp_queries":if dns{8}else{0},"dns_resolver_restored":dns,"owned_program_count":if dns{16}else{8},"policy_map_frozen":freeze,"deferred_desired_head_preserved":freeze,"failed_activation_recorded":freeze,"persistent_restart":true,"owned_programs_released":true,"full_transparent_datapath_ready":false})
    );
}

fn captured_dns(phase: u8) {
    use hickory_proto::op::Message;
    for (family, target, bind) in [
        ("v4", "203.0.113.53:53", "0.0.0.0:0"),
        ("v6", "[2001:db8::53]:53", "[::]:0"),
    ] {
        let target: SocketAddr = target.parse().unwrap();
        let mut query =
            hps_dns::wire::query(&format!("udp-{family}-{phase}.coexist.test"), "A").unwrap();
        query.metadata.id = 700 + phase as u16;
        let socket = std::net::UdpSocket::bind(bind).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket.send_to(&query.to_vec().unwrap(), target).unwrap();
        let mut bytes = vec![0u8; 65535];
        let (n, source) = socket.recv_from(&mut bytes).unwrap();
        assert_eq!(source, target);
        let response = Message::from_vec(&bytes[..n]).unwrap();
        assert_eq!(response.metadata.id, query.metadata.id);
        assert_eq!(
            hps_dns::wire::addresses(&response),
            vec!["203.0.113.9".parse::<std::net::IpAddr>().unwrap()]
        );
        let mut query =
            hps_dns::wire::query(&format!("tcp-{family}-{phase}.coexist.test"), "A").unwrap();
        query.metadata.id = 800 + phase as u16;
        let payload = query.to_vec().unwrap();
        let mut tcp =
            std::net::TcpStream::connect_timeout(&target, Duration::from_secs(5)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(tcp.peer_addr().unwrap(), target);
        tcp.write_all(&(payload.len() as u16).to_be_bytes())
            .unwrap();
        tcp.write_all(&payload).unwrap();
        let mut len = [0; 2];
        tcp.read_exact(&mut len).unwrap();
        let n = u16::from_be_bytes(len) as usize;
        tcp.read_exact(&mut bytes[..n]).unwrap();
        let response = Message::from_vec(&bytes[..n]).unwrap();
        assert_eq!(response.metadata.id, query.metadata.id);
        assert_eq!(
            hps_dns::wire::addresses(&response),
            vec!["203.0.113.9".parse::<std::net::IpAddr>().unwrap()]
        );
    }
}
async fn dns_upstream(
    last: u8,
    count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    jobs: &mut Jobs,
) -> SocketAddr {
    use hickory_proto::{
        op::{Message, OpCode},
        rr::{RData, Record, rdata::A},
    };
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    jobs.0.push(tokio::spawn(async move {
        let mut bytes = [0; 65535];
        loop {
            let (n, peer) = socket.recv_from(&mut bytes).await.unwrap();
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let query = Message::from_vec(&bytes[..n]).unwrap();
            let mut response = Message::response(query.metadata.id, OpCode::Query);
            response.answers.push(Record::from_rdata(
                query.queries[0].name().clone(),
                30,
                RData::A(A::new(203, 0, 113, last)),
            ));
            response.queries = query.queries;
            socket
                .send_to(&response.to_vec().unwrap(), peer)
                .await
                .unwrap();
        }
    }));
    address
}
