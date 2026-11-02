#![cfg(target_os = "linux")]
#[path = "../../platform/tests/support/dns_child.rs"]
mod dns_child;
use hickory_proto::{
    op::{Message, OpCode},
    rr::{RData, Record, rdata::A},
};
use hps_application::{ApplicationError, StartupOverrides, Supervisor};
use hps_config::{DaemonConfig, DnsCaptureConfig, DnsProtocol, DnsSave};
use hps_domain::ConfigActivate;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
#[test]
#[ignore = "helper process invoked by the real kernel application test"]
fn dns_capture_child() {
    dns_child::run();
}
struct Group(PathBuf);
impl Drop for Group {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux and an independently compiled object"]
async fn real_dns_capture() {
    let object = PathBuf::from(std::env::var("HPS_EBPF_OBJECT").expect("HPS_EBPF_OBJECT"));
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap_or_else(|_| "/sys/fs/cgroup".into());
    let group = Group(Path::new(&root).join(format!("hps-dns-test-app-{}", std::process::id())));
    std::fs::create_dir(&group.0).unwrap();
    let upstream = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let address = upstream.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut buffer = vec![0; 65535];
        loop {
            let (len, peer) = upstream.recv_from(&mut buffer).await.unwrap();
            let query = Message::from_vec(&buffer[..len]).unwrap();
            let socket = upstream.clone();
            tokio::spawn(async move {
                if query.queries[0]
                    .name()
                    .to_ascii()
                    .starts_with("reverse-one")
                {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                let mut response = Message::response(query.metadata.id, OpCode::Query);
                response.answers.push(Record::from_rdata(
                    query.queries[0].name().clone(),
                    30,
                    RData::A(A::new(203, 0, 113, 9)),
                ));
                response.queries = query.queries;
                socket
                    .send_to(&response.to_vec().unwrap(), peer)
                    .await
                    .unwrap();
            });
        }
    });
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.dns.upstreams[0].protocol = DnsProtocol::Udp;
    config.dns.upstreams[0].addresses = vec![address];
    config.dns.capture = Some(DnsCaptureConfig {
        object: object.clone(),
        cgroup: group.0.clone(),
        port,
    });
    let service = Supervisor::boot(
        hps_storage::Store::open(None, config.clone()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    service.confirm_startup().await.unwrap();
    let initial = service.dns_overview(None, 50).unwrap();
    assert!(initial.linux_capture_active);
    let original = initial.linux_capture.unwrap();
    let exe = std::env::current_exe().unwrap();
    let scope = group.0.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(exe)
            .args(["--exact", "dns_capture_child", "--ignored", "--nocapture"])
            .env("HPS_DNS_CHILD_CGROUP", scope)
            .stdin(Stdio::null())
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
    let status = service
        .dns_overview(None, 50)
        .unwrap()
        .linux_capture
        .unwrap();
    assert_eq!(status.peers, 4);
    for stats in &status.stats {
        assert!(stats.redirected >= 4);
        assert!(stats.restored >= 4);
        assert_eq!(stats.errors, 0);
    }
    let mut policy = config.dns.clone();
    policy.timeout_ms = 3000;
    assert_eq!(
        service
            .edit_dns(DnsSave {
                configuration: policy.clone(),
                expected_revision: 1
            })
            .await
            .unwrap()
            .state,
        "applied"
    );
    let hot = service.dns_overview(None, 50).unwrap();
    assert_eq!(hot.running_revision, 2);
    assert_eq!(hot.linux_capture.unwrap().program_ids, original.program_ids);
    policy.capture = None;
    let pending = service
        .edit_dns(DnsSave {
            configuration: policy,
            expected_revision: 2,
        })
        .await
        .unwrap();
    assert_eq!(pending.state, "pending_restart");
    assert_eq!(pending.running_revision, 2);
    assert!(pending.restart_required.contains(&"dns.capture".into()));
    assert!(service.dns_overview(None, 50).unwrap().linux_capture_active);
    service
        .rollback(ConfigActivate {
            revision: 2,
            expected_revision: pending.desired_revision,
            defer_until_restart: false,
        })
        .await
        .unwrap();
    let mut bad = config.dns.clone();
    bad.capture.as_mut().unwrap().object = PathBuf::from("/hps-nonexistent-object");
    assert!(matches!(
        service
            .edit_dns(DnsSave {
                configuration: bad,
                expected_revision: 2
            })
            .await,
        Err(ApplicationError::Capture(_))
    ));
    assert_eq!(service.workspace().await.unwrap().desired_revision, 2);
    service.shutdown().await.unwrap();
    for id in &original.program_ids {
        assert!(
            !aya::programs::loaded_programs()
                .filter_map(|p| p.ok())
                .any(|p| p.id() == *id)
        );
    }
    let again = Supervisor::boot(
        hps_storage::Store::open(None, config.clone()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    assert!(again.dns_overview(None, 50).unwrap().linux_capture_active);
    again.shutdown().await.unwrap();
    config.dns.capture = None;
    let plain = Supervisor::boot(
        hps_storage::Store::open(None, config.clone()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    plain.confirm_startup().await.unwrap();
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    config.dns.capture = Some(DnsCaptureConfig {
        object,
        cgroup: group.0.clone(),
        port: occupied.local_addr().unwrap().port(),
    });
    assert!(matches!(
        plain
            .edit_dns(DnsSave {
                configuration: config.dns,
                expected_revision: 1
            })
            .await,
        Err(ApplicationError::Engine(
            hps_engine::runtime::EngineError::Bind
        ))
    ));
    assert_eq!(plain.workspace().await.unwrap().desired_revision, 1);
    assert!(!plain.dns_overview(None, 50).unwrap().linux_capture_active);
    plain.shutdown().await.unwrap();
    task.abort();
    std::fs::remove_dir(&group.0).unwrap();
    println!(
        "HPS_DNS_APPLICATION_EVIDENCE={}",
        serde_json::json!({"kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),
        "capture":status,"boot_and_real_dns":true,"hot_policy_keeps_hooks":true,"capture_disable_pending_restart":true,
        "pending_rollback":true,"invalid_object_keeps_head":true,"bind_failure_keeps_head":true,"shutdown_and_restart_reacquire":true,
        "programs_removed":true,"cgroup_removed":true})
    );
}
