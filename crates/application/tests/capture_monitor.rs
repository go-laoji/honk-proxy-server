#![cfg(target_os = "linux")]
#[path = "../../platform/src/transparent/test_support.rs"]
#[allow(
    dead_code,
    reason = "shared fixtures also contain UDP worker and membership helpers"
)]
mod capture_support;
#[path = "../../platform/src/rtnetlink.rs"]
#[allow(
    dead_code,
    reason = "shared route fixture exposes extra decoded response fields"
)]
mod rtnetlink;
use capture_support::{Address, Group};
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{DaemonConfig, LinuxCaptureConfig};
use hps_domain::RouteAction;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};
#[test]
#[ignore = "helper invoked only by isolated real_capture_monitor_suspends_after_route_loss"]
fn capture_fault_child() {
    let group = std::path::PathBuf::from(std::env::var("HPS_CAPTURE_FAULT_CGROUP").unwrap())
        .canonicalize()
        .unwrap();
    assert!(
        group
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hps-transparent-fault-")
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(group.join("cgroup.procs"))
        .unwrap()
        .write_all(format!("{}\n", std::process::id()).as_bytes())
        .unwrap();
    for target in ["203.0.113.77:37920", "[2001:db8::77]:37920"] {
        let target = target.parse::<std::net::SocketAddr>().unwrap();
        assert_eq!(
            std::net::TcpStream::connect_timeout(&target, Duration::from_secs(3))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let udp = std::net::UdpSocket::bind(if target.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .unwrap();
        assert_eq!(
            udp.send_to(b"must-be-rejected", target).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux and actual eBPF; deletes only an exact owned test rule"]
async fn real_capture_monitor_suspends_after_route_loss() {
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
    let group =
        Group(Path::new(&root).join(format!("hps-transparent-fault-{}", std::process::id())));
    std::fs::create_dir(&group.0).unwrap();
    let _addresses = [Address::add("203.0.113.77"), Address::add("2001:db8::77")];
    let directory = tempfile::tempdir().unwrap();
    let specification = LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        tcp_port: 0,
        udp_port: 0,
        max_flows: 64,
        max_pending: 16,
        max_udp_sessions: 16,
        ..Default::default()
    };
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.dns.enabled = false;
    config.proxy.routing.default_action = RouteAction::Direct;
    config.linux_capture = Some(specification.clone());
    let service = Supervisor::boot(
        hps_storage::Store::open(Some(directory.path()), config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    service.confirm_startup().await.unwrap();
    let initial = service.linux_capture_status().await;
    assert!(initial.ready, "{initial:?}");
    assert_eq!(initial.kernel_revision, Some(1));
    assert_eq!(initial.listeners.len(), 4);
    assert_eq!(initial.program_ids.len(), 8);
    assert_eq!(initial.links_present, Some(true));
    assert_eq!(initial.routes_present, Some(true));
    // Exact capture rule from this fixture identity, no wildcard deletion.
    let mut rule = vec![libc::AF_INET as u8, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];
    for (kind, value) in [
        (6, specification.rule_priority),
        (10, specification.capture_mark),
        (16, u32::MAX),
        (15, specification.routing_table),
    ] {
        rtnetlink::attribute(&mut rule, kind, &value.to_ne_bytes());
    }
    rtnetlink::attribute(&mut rule, 21, &[200]);
    rtnetlink::exchange(
        libc::RTM_DELRULE,
        (libc::NLM_F_REQUEST | libc::NLM_F_ACK) as u16,
        &rule,
        false,
    )
    .unwrap();
    let fault = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = service.linux_capture_status().await;
            if view.suspended && !view.ready && view.state == "degraded" {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fault.routes_present, Some(false));
    assert_eq!(fault.links_present, Some(true));
    assert_eq!(fault.kernel_revision, Some(1));
    let group_path = group.0.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "capture_fault_child", "--ignored", "--nocapture"])
            .env("HPS_CAPTURE_FAULT_CGROUP", group_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let denied = service.linux_capture_status().await;
    assert_eq!(
        denied.hooks.iter().map(|h| h.rejected_packets).sum::<u64>(),
        4
    );
    assert_eq!(
        service
            .current()
            .engine
            .node_registry()
            .connection_statistics()
            .started,
        0
    );
    service.shutdown().await.unwrap();
    let stopped = service.linux_capture_status().await;
    assert!(!stopped.ready);
    assert_eq!(stopped.state, "stopped");
    assert!(stopped.program_ids.is_empty());
    let restarted = Supervisor::boot(
        hps_storage::Store::open(Some(directory.path()), DaemonConfig::default()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    restarted.confirm_startup().await.unwrap();
    let recovered = restarted.linux_capture_status().await;
    assert!(recovered.ready, "{recovered:?}");
    assert!(!recovered.suspended);
    assert_eq!(recovered.kernel_revision, Some(1));
    restarted.shutdown().await.unwrap();
    println!(
        "{}",
        serde_json::json!({"scope":"daemon_capture_monitor_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":initial.object_fingerprint,"initial_ready_verified":true,"route_loss_detected":true,"fault_suspended":true,"ipv4_tcp_udp_denied":true,"ipv6_tcp_udp_denied":true,"kernel_rejections":4,"stopped_not_ready":true,"persistent_restart_recovered":true,"full_transparent_datapath_ready":false})
    );
}
