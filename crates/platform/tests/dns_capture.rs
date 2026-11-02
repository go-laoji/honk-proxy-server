#![cfg(target_os = "linux")]
use hickory_proto::{
    op::{Message, OpCode},
    rr::{RData, Record, rdata::A},
};
use hps_config::{DnsConfig, DnsProtocol, ProxyConfig};
use hps_dns::{Resolver, server::DnsServer};
use hps_platform::dns_capture::{IPV6_ALIAS_PREFIX, PreparedCapture};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

#[path = "support/dns_child.rs"]
mod dns_child;
#[test]
#[ignore = "helper process invoked only by real_dns_capture"]
fn dns_capture_child() {
    dns_child::run();
}
struct Group(PathBuf);
impl Drop for Group {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
struct Membership(PathBuf);
impl Membership {
    fn enter(group: &Path) -> Self {
        let previous = std::fs::read_to_string("/proc/self/cgroup").unwrap();
        let previous = previous
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .unwrap();
        assert!(
            !Path::new(previous)
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        );
        let previous = Path::new("/sys/fs/cgroup").join(previous.trim_start_matches('/'));
        Self::write(group);
        Self(previous)
    }
    fn write(group: &Path) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(group.join("cgroup.procs"))
            .unwrap()
            .write_all(format!("{}\n", std::process::id()).as_bytes())
            .unwrap();
    }
}
impl Drop for Membership {
    fn drop(&mut self) {
        Self::write(&self.0);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires real privileged Linux, writable cgroup v2 and an independently compiled object"]
async fn real_dns_capture() {
    let object = std::env::var("HPS_EBPF_OBJECT").expect("HPS_EBPF_OBJECT");
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap_or_else(|_| "/sys/fs/cgroup".into());
    let group = Group(Path::new(&root).join(format!("hps-dns-test-{}", std::process::id())));
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
    let mut config = DnsConfig::default();
    config.upstreams[0].protocol = DnsProtocol::Udp;
    config.upstreams[0].addresses = vec![address];
    let resolver = Resolver::prepare(config, &ProxyConfig::default(), None).unwrap();
    let v4 = DnsServer::bind_capture("0.0.0.0:0".parse().unwrap(), resolver.clone())
        .await
        .unwrap();
    let port = v4.local_address().port();
    let v6 = DnsServer::bind_capture(format!("[::]:{port}").parse().unwrap(), resolver)
        .await
        .unwrap();
    let capture = PreparedCapture::read(Path::new(&object), &group.0, port)
        .unwrap()
        .load()
        .unwrap();
    v4.start();
    v6.start();
    capture.activate().unwrap();
    let membership = Membership::enter(&group.0);
    let sink = tokio::net::UdpSocket::bind("127.0.0.1:53").await.unwrap();
    let own = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    own.send_to(b"hps-daemon-bypass", sink.local_addr().unwrap())
        .await
        .unwrap();
    let mut proof = [0; 64];
    let len = tokio::time::timeout(Duration::from_secs(2), sink.recv(&mut proof))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&proof[..len], b"hps-daemon-bypass");
    drop(sink);
    drop(own);
    assert!(
        PreparedCapture::read(Path::new(&object), &group.0, port)
            .unwrap()
            .load()
            .is_err()
    );
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
    let status = capture.status().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}\nCapture: {status:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(status.attached);
    assert_eq!(status.program_ids.len(), 8);
    assert!(status.map_ids.len() >= 5);
    assert_eq!(status.peers, 4);
    for stats in &status.stats {
        assert!(stats.redirected >= 4);
        assert!(stats.restored >= 4);
        assert_eq!(stats.errors, 0);
    }
    assert!(status.stats.iter().any(|stats| stats.bypassed > 0));
    capture.shutdown().unwrap();
    v4.shutdown().await;
    v6.shutdown().await;
    for id in &status.program_ids {
        assert!(
            !aya::programs::loaded_programs()
                .filter_map(|p| p.ok())
                .any(|p| p.id() == *id)
        );
    }
    let started = std::time::Instant::now();
    loop {
        if status.map_ids.iter().all(|id| matches!(aya::maps::MapInfo::from_id(*id), Err(aya::maps::MapError::SyscallError(ref e)) if e.io_error.raw_os_error() == Some(libc::ENOENT))) { break; }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "owned DNS maps were not reclaimed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let next = PreparedCapture::read(Path::new(&object), &group.0, port)
        .unwrap()
        .load()
        .unwrap();
    next.shutdown().unwrap();
    task.abort();
    drop(membership);
    std::fs::remove_dir(&group.0).unwrap();
    println!(
        "HPS_DNS_CAPTURE_EVIDENCE={}",
        serde_json::json!({"kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),
        "capture":status,"ipv4_udp_tcp":true,"ipv6_udp_tcp":true,"parallel_udp_peer_restoration":true,"getpeername_restored":true,"daemon_pid_namespace_exclusion":true,
        "programs_removed":true,"maps_removed":true,"route_removed":true,"namespace_ownership_reacquired":true,"cgroup_removed":true,
        "ipv6_owned_route":format!("{IPV6_ALIAS_PREFIX}/112"),"full_transparent_datapath_ready":false})
    );
}
