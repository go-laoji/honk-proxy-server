#![cfg(target_os = "linux")]
use hps_config::LinuxCaptureConfig;
use hps_ebpf_abi::{FlowKey, TP_LOCAL, TP_PROCESS_VERIFIED};
use hps_platform::transparent::{Capture, CaptureSockets, PreparedCapture};
use std::{
    io::{BufRead, Read, Write},
    net::SocketAddr,
    os::fd::AsFd,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "support/transparent_child.rs"]
mod child;
#[test]
#[ignore = "helper process invoked only by real_transparent_original_packets"]
fn transparent_capture_child() {
    child::run();
}
struct Group(PathBuf);
impl Drop for Group {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn words(address: std::net::IpAddr) -> [u32; 4] {
    match address {
        std::net::IpAddr::V4(address) => [u32::from_ne_bytes(address.octets()), 0, 0, 0],
        std::net::IpAddr::V6(address) => {
            let bytes = address.octets();
            std::array::from_fn(|i| u32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()))
        }
    }
}
fn verified_process(
    capture: &Capture,
    source: SocketAddr,
    destination: SocketAddr,
    protocol: u32,
) -> u32 {
    let key = FlowKey {
        family: if source.is_ipv4() { 4 } else { 6 },
        protocol,
        source: words(source.ip()),
        destination: words(destination.ip()),
        source_port: u32::from(source.port()),
        destination_port: u32::from(destination.port()),
        netns_cookie: capture.netns_cookie().unwrap(),
    };
    let flow = capture
        .flow(key)
        .unwrap()
        .expect("actual captured tuple must have metadata");
    assert_eq!(flow.revision, 7);
    assert_eq!(flow.origin, TP_LOCAL);
    assert_eq!(flow.flags & TP_PROCESS_VERIFIED, TP_PROCESS_VERIFIED);
    assert!(flow.process.socket_cookie > 0 && flow.process.cgroup_id > 0 && flow.process.pid > 0);
    flow.process.pid
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux, private writable cgroup v2 and actual eBPF object"]
async fn real_transparent_original_packets() {
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
    let group =
        Group(Path::new(&root).join(format!("hps-transparent-packets-{}", std::process::id())));
    std::fs::create_dir(&group.0).unwrap();
    let config = LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        tcp_port: 0,
        udp_port: 0,
        ..Default::default()
    };
    let tcp4 =
        hps_net::transparent::bind_tcp("0.0.0.0:0".parse().unwrap(), config.bypass_mark, 128)
            .unwrap();
    let tcp6 =
        hps_net::transparent::bind_tcp("[::]:0".parse().unwrap(), config.bypass_mark, 128).unwrap();
    let udp4 =
        hps_net::transparent::bind_udp("0.0.0.0:0".parse().unwrap(), config.bypass_mark).unwrap();
    let udp6 =
        hps_net::transparent::bind_udp("[::]:0".parse().unwrap(), config.bypass_mark).unwrap();
    let capture = Arc::new(
        PreparedCapture::read(&config)
            .unwrap()
            .load(CaptureSockets {
                tcp4: Some(tcp4.as_fd()),
                tcp6: Some(tcp6.as_fd()),
                udp4: Some(udp4.as_fd()),
                udp6: Some(udp6.as_fd()),
            })
            .unwrap(),
    );
    assert!(capture.activate(7, true).is_err());
    capture.prepare_routes().unwrap();
    let pids = Arc::new(Mutex::new(vec![]));
    let mut jobs = vec![];
    for listener in [tcp4, tcp6] {
        let capture = capture.clone();
        let pids = pids.clone();
        jobs.push(tokio::spawn(async move {
            let (mut stream, source) = listener.accept().await.unwrap();
            let destination = stream.local_addr().unwrap();
            assert_eq!(destination.port(), 8443);
            assert!(
                destination.ip() == "203.0.113.77".parse::<std::net::IpAddr>().unwrap()
                    || destination.ip() == "2001:db8::77".parse::<std::net::IpAddr>().unwrap()
            );
            pids.lock()
                .unwrap()
                .push(verified_process(&capture, source, destination, 6));
            let mut message = [0; 19];
            stream.read_exact(&mut message).await.unwrap();
            assert_eq!(&message, b"tcp-original-target");
            stream.write_all(b"TCP-original-target").await.unwrap();
        }));
    }
    for socket in [udp4, udp6] {
        let capture = capture.clone();
        let pids = pids.clone();
        let mark = config.bypass_mark;
        jobs.push(tokio::spawn(async move {
            let mut buffer = [0; 128];
            for _ in 0..5 {
                let packet = hps_net::transparent::recv_original(&socket, &mut buffer)
                    .await
                    .unwrap();
                assert!([5353, 5354].contains(&packet.destination.port()));
                assert_eq!(&buffer[..packet.bytes], b"udp-original-target");
                pids.lock().unwrap().push(verified_process(
                    &capture,
                    packet.source,
                    packet.destination,
                    17,
                ));
                let reply =
                    hps_net::transparent::bind_udp_reply(packet.destination, packet.source, mark)
                        .unwrap();
                reply.send(b"UDP-original-target").await.unwrap();
            }
        }));
    }
    let exe = std::env::current_exe().unwrap();
    let scope = group.0.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (enable_tx, enable_rx) = std::sync::mpsc::channel();
    let child_task = tokio::task::spawn_blocking(move || {
        let mut child = Command::new(exe)
            .args([
                "--exact",
                "transparent_capture_child",
                "--ignored",
                "--nocapture",
            ])
            .env("HPS_TRANSPARENT_CHILD_CGROUP", scope)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut before = String::new();
        loop {
            let mut line = String::new();
            if stdout.read_line(&mut line).unwrap() == 0 {
                break;
            }
            before.push_str(&line);
            assert!(before.len() <= 4096);
            if line.contains("HPS_TRANSPARENT_CHILD_READY") {
                break;
            }
        }
        ready_tx.send(()).unwrap();
        if enable_rx.recv_timeout(Duration::from_secs(5)).is_ok() {
            child.stdin.take().unwrap().write_all(&[1]).unwrap();
        } else {
            let _ = child.kill();
        }
        let mut remaining = vec![];
        stdout.read_to_end(&mut remaining).unwrap();
        let mut output = child.wait_with_output().unwrap();
        output.stdout = before.into_bytes();
        output.stdout.extend_from_slice(&remaining);
        (pid, output)
    });
    tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .unwrap()
        .unwrap();
    capture.activate(7, true).unwrap();
    enable_tx.send(()).unwrap();
    let (pid, output) = child_task.await.unwrap();
    let stats = capture.stats().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}\nstats={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        stats
            .iter()
            .map(|s| (
                s.observed_sockets,
                s.marked_sockets,
                s.captured_packets,
                s.missing_process,
                s.errors,
                s.last_error,
                s.error_phase
            ))
            .collect::<Vec<_>>()
    );
    for job in jobs {
        tokio::time::timeout(Duration::from_secs(2), job)
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(pids.lock().unwrap().as_slice(), &[pid; 12]);
    assert!(stats[0].marked_sockets >= 12 && stats[2].captured_packets >= 12);
    assert_eq!(stats[0].rejected_packets, 2);
    assert!(
        stats
            .iter()
            .all(|s| s.errors == 0 && s.missing_process == 0)
    );
    assert!(capture.routes_present().unwrap() && capture.links_present().unwrap());
    capture.shutdown().unwrap();
    assert!(!capture.routes_present().unwrap() && !capture.links_present().unwrap());
    println!(
        "{}",
        serde_json::json!({"scope":"original_packet_capture_and_reply_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":capture.fingerprint,"revision":7,"ipv4_tcp_udp":true,"ipv6_tcp_udp":true,"original_tcp_destination":true,"udp_original_reply_source":true,"connected_and_unconnected_udp":true,"preexisting_udp_first_send_rejected":2,"process_pid":pid,"verified_flows":12,"forwarding_verified":false})
    );
}
