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
use hps_domain::{LinuxCaptureAction, LinuxCaptureRequest, RouteAction};
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
    let rules = service.rules_overview(None).await.unwrap().capabilities;
    assert!(
        rules.local_process_available && rules.udp_data_plane_available && rules.kernel_rules_ready
    );
    assert_eq!(rules.kernel_rule_revision, Some(1));
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
    let degraded_rules = service.rules_overview(None).await.unwrap().capabilities;
    assert!(!degraded_rules.local_process_available && !degraded_rules.kernel_rules_ready);
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
    let recovered_in_place = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let view = service.linux_capture_status().await;
            if view.ready {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(recovered_in_place.retry_attempts, 1);
    assert_eq!(recovered_in_place.program_ids, initial.program_ids);
    assert_eq!(recovered_in_place.map_ids, initial.map_ids);
    assert_eq!(recovered_in_place.routes_present, Some(true));
    assert!(!recovered_in_place.suspended);
    // A replaced scope must never be silently replaced or captured as a new
    // identity. Retry exhaustion stays degraded even if the old name returns.
    let overlay = Group(group.0.with_extension("identity-overlay"));
    std::fs::create_dir(&overlay.0).unwrap();
    let source = std::ffi::CString::new(overlay.0.as_os_str().as_encoded_bytes()).unwrap();
    let target = std::ffi::CString::new(group.0.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(
        unsafe {
            libc::mount(
                source.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND,
                std::ptr::null(),
            )
        },
        0
    );
    struct Restore(std::ffi::CString);
    impl Drop for Restore {
        fn drop(&mut self) {
            assert_eq!(
                unsafe { libc::umount2(self.0.as_ptr(), libc::MNT_DETACH) },
                0
            );
        }
    }
    let restore = Restore(target);
    let exhausted = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let view = service.linux_capture_status().await;
            if view.retry_attempts == 3 && view.state == "degraded" {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    assert!(exhausted.suspended && !exhausted.ready);
    assert_eq!(exhausted.max_retry_attempts, 3);
    drop(restore);
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(!service.linux_capture_status().await.ready);
    let request = |action| LinuxCaptureRequest {
        expected_epoch: service.event_epoch().into(),
        expected_revision: 1,
        action,
    };
    let mut stale = request(LinuxCaptureAction::Stop);
    stale.expected_epoch = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        service.control_linux_capture(stale).await,
        Err(hps_application::ApplicationError::ConnectionEpoch)
    ));
    let checked = service
        .control_linux_capture(request(LinuxCaptureAction::Recheck))
        .await
        .unwrap();
    assert!(checked.ready, "{checked:?}");
    let explicit_before = service.current().engine.status().inbounds;
    let stopped_by_user = service
        .control_linux_capture(request(LinuxCaptureAction::Stop))
        .await
        .unwrap();
    assert_eq!(stopped_by_user.state, "stopped");
    assert!(!stopped_by_user.ready);
    assert!(stopped_by_user.program_ids.is_empty());
    let stopped_rules = service.rules_overview(None).await.unwrap().capabilities;
    assert!(
        !stopped_rules.local_process_available
            && !stopped_rules.udp_data_plane_available
            && !stopped_rules.kernel_rules_ready
    );
    assert_eq!(stopped_rules.kernel_rule_revision, None);
    assert_eq!(
        explicit_before.iter().filter(|i| i.running).count(),
        service
            .current()
            .engine
            .status()
            .inbounds
            .iter()
            .filter(|i| i.running)
            .count()
    );
    let started = service
        .control_linux_capture(request(LinuxCaptureAction::Start))
        .await
        .unwrap();
    assert!(started.ready, "{started:?}");
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
        serde_json::json!({"scope":"daemon_capture_monitor_and_repair_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":initial.object_fingerprint,"initial_ready_verified":true,"route_loss_detected":true,"fault_suspended":true,"ipv4_tcp_udp_denied":true,"ipv6_tcp_udp_denied":true,"kernel_rejections":4,"in_place_repaired_same_programs_and_maps":true,"identity_change_refused":true,"bounded_retries":3,"manual_recheck_recovered":true,"stale_epoch_refused":true,"user_stop_start_verified":true,"explicit_inbounds_preserved":true,"stopped_not_ready":true,"persistent_restart_recovered":true,"full_transparent_datapath_ready":false})
    );
}
