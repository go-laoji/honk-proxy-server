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
    for phase in 0..3u8 {
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
        if phase > 0 {
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
        if phase < 2 {
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
            for _ in 0..3 {
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
            for _ in 0..3 {
                let (n, peer) = socket.recv_from(&mut bytes).await.unwrap();
                socket.send_to(&bytes[..n], peer).await.unwrap();
            }
        }));
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
                && f.bytes_up == 6
                && f.bytes_down == 6)
    );
    for revision in [second, third] {
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
    assert_eq!(owned.len(), 8);
    service.shutdown().await.unwrap();
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
    assert_eq!(restarted.workspace().await.unwrap().running_revision, third);
    assert_eq!(restarted.current().engine.generation(), third);
    restarted.shutdown().await.unwrap();
    println!(
        "{}",
        serde_json::json!({"scope":"daemon_transparent_local_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"client_pid":pid,"ipv4":true,"ipv6":true,"tcp":true,"udp":true,"actual_process_rule":true,"old_revision":1,"reject_revision":second,"name_only_revision":third,"old_connections_retained":4,"new_connections_rejected":8,"business_bytes_up":24,"business_bytes_down":24,"persistent_restart":true,"owned_programs_released":true,"full_transparent_datapath_ready":false})
    );
}
