#![cfg(target_os = "linux")]
#[path = "../../../crates/platform/src/transparent/test_support.rs"]
#[allow(
    dead_code,
    reason = "shared fixtures include unrelated capture helpers"
)]
mod capture_support;
#[path = "../../../crates/platform/src/rtnetlink.rs"]
#[allow(dead_code, reason = "shared RTNL exposes extra inspection helpers")]
mod rtnetlink;
use capture_support::{Address, Group, Jobs, Membership};
use hps_client::HpsClient;
use hps_config::{DaemonConfig, LinuxCaptureConfig};
use hps_domain::{LinuxCaptureOverview, RouteAction};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
struct Process {
    child: Child,
    log: PathBuf,
}
impl Process {
    fn start(config: &Path, data: &Path, log: PathBuf) -> Self {
        let binary = std::env::var_os("HPS_DAEMON_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_hpsd").into());
        let child = Command::new(binary)
            .arg("--config")
            .arg(config)
            .arg("--data-dir")
            .arg(data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(std::fs::File::create(&log).unwrap()))
            .spawn()
            .unwrap();
        Self { child, log }
    }
    async fn wait(&mut self) -> std::process::ExitStatus {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "hpsd exit timeout: {}",
                std::fs::read_to_string(&self.log).unwrap()
            )
        })
    }
    async fn ready(&mut self, client: &HpsClient) -> LinuxCaptureOverview<LinuxCaptureConfig> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                assert!(
                    self.child.try_wait().unwrap().is_none(),
                    "hpsd exited before ready: {}",
                    std::fs::read_to_string(&self.log).unwrap()
                );
                if let Ok(view) = client.linux_capture().await
                    && view.status.ready
                {
                    return view;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "hpsd ready timeout: {}",
                std::fs::read_to_string(&self.log).unwrap()
            )
        })
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct ForeignRule {
    payload: Vec<u8>,
    priority: u32,
    mark: u32,
    table: u32,
}
impl ForeignRule {
    fn create(priority: u32, mark: u32, table: u32) -> Self {
        let mut payload = vec![libc::AF_INET as u8, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];
        for (kind, value) in [(6, priority), (10, mark), (16, u32::MAX), (15, table)] {
            rtnetlink::attribute(&mut payload, kind, &value.to_ne_bytes());
        }
        rtnetlink::attribute(&mut payload, 21, &[201]);
        rtnetlink::exchange(
            libc::RTM_NEWRULE,
            (libc::NLM_F_REQUEST | libc::NLM_F_ACK | libc::NLM_F_CREATE | libc::NLM_F_EXCL) as u16,
            &payload,
            false,
        )
        .unwrap();
        Self {
            payload,
            priority,
            mark,
            table,
        }
    }
    fn present(&self) -> bool {
        rule_present(self.priority, self.mark, self.table, 201)
    }
}
impl Drop for ForeignRule {
    fn drop(&mut self) {
        let _ = rtnetlink::exchange(
            libc::RTM_DELRULE,
            (libc::NLM_F_REQUEST | libc::NLM_F_ACK) as u16,
            &self.payload,
            false,
        );
    }
}
fn rule_present(priority: u32, mark: u32, table: u32, protocol: u8) -> bool {
    let mut payload = [0; 12];
    payload[0] = libc::AF_INET as u8;
    rtnetlink::exchange(
        libc::RTM_GETRULE,
        (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16,
        &payload,
        true,
    )
    .unwrap()
    .iter()
    .any(|m| {
        if m.kind != libc::RTM_NEWRULE || m.payload.len() < 12 {
            return false;
        }
        let attrs = rtnetlink::attributes(&m.payload, 12).unwrap();
        [(6, priority), (10, mark), (15, table)]
            .into_iter()
            .all(|(key, value)| attrs.get(&key).is_some_and(|v| v == &value.to_ne_bytes()))
            && attrs.get(&21).is_some_and(|v| v == &[protocol])
    })
}
async fn traffic(group: &Path, targets: &[SocketAddr]) {
    let _membership = Membership::enter(group);
    let mut tcp = tokio::net::TcpStream::connect(targets[0]).await.unwrap();
    tcp.write_all(b"crash-tcp").await.unwrap();
    let mut reply = [0; 9];
    tokio::time::timeout(Duration::from_secs(3), tcp.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"crash-tcp");
    let udp = tokio::net::UdpSocket::bind("[::]:0").await.unwrap();
    udp.send_to(b"crash-udp", targets[1]).await.unwrap();
    let mut reply = [0; 64];
    let (n, source) = tokio::time::timeout(Duration::from_secs(3), udp.recv_from(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source, targets[1]);
    assert_eq!(&reply[..n], b"crash-udp");
}
async fn verified_flows(client: &HpsClient) {
    let view = client
        .connections(&hps_domain::ConnectionQuery {
            state: Some("all".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(view.statistics.started, 2);
    assert_eq!(view.records.len(), 2);
    for flow in view.records {
        assert_eq!(flow.context.origin, hps_domain::FlowOrigin::LinuxLocal);
        assert_eq!(flow.context.process.unwrap().pid, std::process::id());
        assert_eq!(flow.routing.configuration_revision, 1);
        assert_eq!((flow.bytes_up, flow.bytes_down), (9, 9));
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires actual standalone hpsd, isolated privileged Linux, cgroup v2 and real eBPF; SIGKILLs only fixture children"]
async fn real_hpsd_sigkill_recovery_preserves_foreign_rules() {
    use std::os::unix::process::ExitStatusExt;
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
    let group =
        Group(Path::new(&root).join(format!("hps-transparent-crash-{}", std::process::id())));
    std::fs::create_dir(&group.0).unwrap();
    let _addresses = [Address::add("203.0.113.77"), Address::add("2001:db8::77")];
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let config_path = directory.path().join("daemon.toml");
    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let management = reserve.local_addr().unwrap();
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
    config.management_listen = management;
    config.proxy.routing.default_action = RouteAction::Direct;
    config.linux_capture = Some(specification.clone());
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    drop(reserve);
    let client = HpsClient::new(&format!("http://{management}"), Duration::from_secs(1)).unwrap();
    let foreign = ForeignRule::create(
        specification.rule_priority + 10,
        0x1111_1212,
        specification.routing_table + 1,
    );
    let mut jobs = Jobs(vec![]);
    let mut targets = vec![];
    let listener = tokio::net::TcpListener::bind("203.0.113.77:0")
        .await
        .unwrap();
    targets.push(listener.local_addr().unwrap());
    jobs.0.push(tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 9];
            stream.read_exact(&mut bytes).await.unwrap();
            stream.write_all(&bytes).await.unwrap();
        }
    }));
    let socket = hps_net::transparent::bind_udp(
        "[2001:db8::77]:0".parse().unwrap(),
        specification.bypass_mark,
    )
    .unwrap();
    targets.push(socket.local_addr().unwrap());
    jobs.0.push(tokio::spawn(async move {
        for _ in 0..2 {
            let mut bytes = [0; 64];
            let (n, peer) = socket.recv_from(&mut bytes).await.unwrap();
            socket.send_to(&bytes[..n], peer).await.unwrap();
        }
    }));
    let mut first = Process::start(&config_path, &data, directory.path().join("first.log"));
    let initial = first.ready(&client).await;
    assert!(client.status().await.unwrap().proxy_ready);
    let ready: serde_json::Value = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://{management}/readyz"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ready["transparent_ready"], true);
    assert_eq!(ready["explicit_ready"], false);
    traffic(&group.0, &targets).await;
    verified_flows(&client).await;
    first.child.kill().unwrap();
    assert_eq!(first.wait().await.signal(), Some(libc::SIGKILL));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let programs = aya::programs::loaded_programs()
                .take(65537)
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(programs.len() <= 65536);
            if !programs
                .iter()
                .any(|program| initial.status.program_ids.contains(&program.id()))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(rule_present(
        specification.rule_priority,
        specification.capture_mark,
        specification.routing_table,
        200
    ));
    assert!(foreign.present());
    let conflict = ForeignRule::create(
        specification.rule_priority,
        0x2222_2323,
        specification.routing_table + 2,
    );
    let mut refused = Process::start(&config_path, &data, directory.path().join("refused.log"));
    assert!(!refused.wait().await.success());
    assert!(conflict.present());
    assert!(foreign.present());
    drop(conflict);
    let mut second = Process::start(&config_path, &data, directory.path().join("second.log"));
    let recovered = second.ready(&client).await;
    assert_ne!(initial.epoch, recovered.epoch);
    assert_eq!(recovered.running_revision, 1);
    assert_eq!(recovered.status.kernel_revision, Some(1));
    assert!(foreign.present());
    assert!(client.status().await.unwrap().proxy_ready);
    traffic(&group.0, &targets).await;
    verified_flows(&client).await;
    assert_eq!(
        unsafe { libc::kill(second.child.id() as i32, libc::SIGTERM) },
        0
    );
    assert!(second.wait().await.success());
    assert!(foreign.present());
    assert!(!rule_present(
        specification.rule_priority,
        specification.capture_mark,
        specification.routing_table,
        200
    ));
    for job in std::mem::take(&mut jobs.0) {
        tokio::time::timeout(Duration::from_secs(3), job)
            .await
            .unwrap()
            .unwrap();
    }
    println!(
        "{}",
        serde_json::json!({"scope":"standalone_hpsd_sigkill_recovery_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":initial.status.object_fingerprint,"actual_process_killed":true,"signal":9,"old_programs_released":true,"owned_route_residue_detected":true,"foreign_rule_preserved":true,"foreign_priority_conflict_refused":true,"persisted_revision_recovered":1,"epoch_changed":true,"real_tcp_udp_before_and_after_restart":true,"readyz_requires_actual_capture_and_forwarding":true,"graceful_shutdown_owned_rules_removed":true,"full_transparent_datapath_ready":false})
    );
}
