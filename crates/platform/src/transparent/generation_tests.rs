use super::{PreparedCapture, test_support::Group};
use aya::maps::{HashMap, Map, MapData};
use hps_config::LinuxCaptureConfig;
use hps_ebpf_abi::{CapturePolicy, TP_ABI_VERSION};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering},
};

#[test]
#[ignore = "requires actual privileged Linux and an independently compiled eBPF object"]
fn real_policy_publication_observes_only_complete_snapshots() {
    let group = Group(
        std::path::PathBuf::from(std::env::var("HPS_TEST_CGROUP_ROOT").unwrap())
            .join(format!("hps-transparent-policy-{}", std::process::id())),
    );
    std::fs::create_dir(&group.0).unwrap();
    let specification = LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        ..Default::default()
    };
    // Verify the actual object and all programs, but do not attach any links or
    // routes. Deliberately distinct fields exercise all 56 bytes under writes.
    let prepared = PreparedCapture::read(&specification).unwrap();
    let (mut bpf, first) = prepared.prepare_bpf().unwrap();
    let second = CapturePolicy {
        abi_version: TP_ABI_VERSION,
        flags: u32::MAX,
        revision: i64::MAX as u64,
        netns_cookie: !first.netns_cookie,
        pidns_device: !first.pidns_device,
        pidns_inode: !first.pidns_inode,
        excluded_tgid: !first.excluded_tgid,
        capture_mark: !first.capture_mark,
        bypass_mark: !first.bypass_mark,
        reserved: u32::MAX,
    };
    let id = match bpf.map("HPS_TP_POLICY").unwrap() {
        Map::HashMap(map) => map.info().unwrap().id(),
        _ => panic!("policy is not a hash"),
    };
    let barrier = Arc::new(Barrier::new(3));
    let complete = AtomicBool::new(false);
    let counts = std::thread::scope(|scope| {
        let mut readers = vec![];
        for _ in 0..2 {
            let barrier = barrier.clone();
            let complete = &complete;
            readers.push(scope.spawn(move || {
                let map = HashMap::<_, u32, CapturePolicy>::try_from(Map::HashMap(
                    MapData::from_id(id).unwrap(),
                ))
                .unwrap();
                barrier.wait();
                let mut reads = 0u64;
                while !complete.load(Ordering::Acquire) || reads < 10_000 {
                    let value = map.get(&0, 0).unwrap();
                    assert!(value == first || value == second, "torn policy: {value:?}");
                    reads += 1;
                }
                reads
            }));
        }
        let mut map =
            HashMap::<_, u32, CapturePolicy>::try_from(bpf.map_mut("HPS_TP_POLICY").unwrap())
                .unwrap();
        barrier.wait();
        for i in 0..10_000 {
            map.insert(0, if i % 2 == 0 { second } else { first }, 2)
                .unwrap();
        }
        complete.store(true, Ordering::Release);
        readers
            .into_iter()
            .map(|r| r.join().unwrap())
            .collect::<Vec<_>>()
    });
    println!(
        "{}",
        serde_json::json!({
            "scope": "kernel_policy_publication_only",
            "kernel": std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),
            "object_sha256": prepared.fingerprint,
            "abi_version": TP_ABI_VERSION,
            "updates": 10_000,
            "reader_lookups": counts,
            "torn_snapshots": 0,
            "full_transparent_datapath_ready": false,
        })
    );
}

#[test]
#[ignore = "helper invoked only by isolated real_flow_identity_and_generation_retirement"]
fn flow_generation_child() {
    use std::io::{Read, Write};
    fn client(address: std::net::SocketAddr) -> std::net::UdpSocket {
        let socket = socket2::Socket::new(
            socket2::Domain::for_address(address),
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )
        .unwrap();
        socket.set_reuse_address(true).unwrap();
        if address.is_ipv6() {
            socket.set_only_v6(true).unwrap();
        }
        socket.bind(&address.into()).unwrap();
        socket.into()
    }
    let group = std::path::PathBuf::from(std::env::var("HPS_FLOW_CGROUP").unwrap())
        .canonicalize()
        .unwrap();
    assert!(
        group
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hps-transparent-generation-")
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(group.join("cgroup.procs"))
        .unwrap()
        .write_all(format!("{}\n", std::process::id()).as_bytes())
        .unwrap();
    let mut control =
        std::os::unix::net::UnixStream::connect(std::env::var("HPS_FLOW_CONTROL").unwrap())
            .unwrap();
    control
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    for target in ["203.0.113.77:37917", "[2001:db8::77]:37917"] {
        let target: std::net::SocketAddr = target.parse().unwrap();
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_NAME, c"hps-first-send".as_ptr(), 0, 0, 0) },
            0
        );
        let socket = client(
            if target.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            }
            .parse()
            .unwrap(),
        );
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        for phase in 0..4u8 {
            if phase == 1 {
                assert_eq!(
                    unsafe { libc::prctl(libc::PR_SET_NAME, c"hps-renamed".as_ptr(), 0, 0, 0) },
                    0
                );
            }
            socket.send_to(&[phase], target).unwrap();
            let mut bytes = [0; 4];
            let (n, source) = socket.recv_from(&mut bytes).unwrap();
            assert_eq!((n, source, bytes[0]), (1, target, phase));
            if phase == 2 {
                // Bind a second independently created cookie to the same tuple.
                let conflict = client(socket.local_addr().unwrap());
                conflict.send_to(&[99], target).unwrap();
                control.write_all(b"X").unwrap();
                control.read_exact(&mut bytes[..1]).unwrap();
                assert_eq!(bytes[0], b'Y');
            }
        }
    }
    let socket = client("0.0.0.0:0".parse().unwrap());
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    for port in 38000..38064 {
        let target = std::net::SocketAddr::new("203.0.113.77".parse().unwrap(), port);
        socket.send_to(b"C", target).unwrap();
        let mut byte = [0];
        let (n, source) = socket.recv_from(&mut byte).unwrap();
        assert_eq!((n, source, byte[0]), (1, target, b'C'));
    }
    let target = "203.0.113.77:38064"
        .parse::<std::net::SocketAddr>()
        .unwrap();
    socket.send_to(b"F", target).unwrap();
    control.write_all(b"F").unwrap();
    let mut byte = [0];
    control.read_exact(&mut byte).unwrap();
    assert_eq!(byte[0], b'G');
    // Only the application retries its dropped datagram after exact retirement.
    // The service does not replay any datagram.
    socket.send_to(b"H", target).unwrap();
    let (n, source) = socket.recv_from(&mut byte).unwrap();
    assert_eq!((n, source, byte[0]), (1, target, b'H'));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires isolated privileged Linux, writable cgroup v2 and actual eBPF object"]
async fn real_flow_identity_and_generation_retirement() {
    use super::{CaptureSockets, test_support::Address};
    use std::{
        os::fd::AsFd,
        process::{Command, Stdio},
        time::Duration,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
    let group = Group(
        std::path::Path::new(&root)
            .join(format!("hps-transparent-generation-{}", std::process::id())),
    );
    std::fs::create_dir(&group.0).unwrap();
    let _addresses = [Address::add("203.0.113.77"), Address::add("2001:db8::77")];
    let specification = LinuxCaptureConfig {
        object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
        local_cgroup: Some(group.0.clone()),
        tcp: false,
        udp_port: 0,
        max_flows: 64,
        max_pending: 16,
        max_udp_sessions: 16,
        ..Default::default()
    };
    let v4 =
        hps_net::transparent::bind_udp("0.0.0.0:0".parse().unwrap(), specification.bypass_mark)
            .unwrap();
    let v6 = hps_net::transparent::bind_udp("[::]:0".parse().unwrap(), specification.bypass_mark)
        .unwrap();
    let capture = PreparedCapture::read(&specification)
        .unwrap()
        .load(CaptureSockets {
            tcp4: None,
            tcp6: None,
            udp4: Some(v4.as_fd()),
            udp6: Some(v6.as_fd()),
        })
        .unwrap();
    capture.prepare_routes().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("control.sock");
    let control = tokio::net::UnixListener::bind(&path).unwrap();
    capture.activate(7, true).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "transparent::generation_tests::flow_generation_child",
            "--ignored",
            "--nocapture",
        ])
        .env("HPS_FLOW_CGROUP", &group.0)
        .env("HPS_FLOW_CONTROL", &path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let (mut control, _) = tokio::time::timeout(Duration::from_secs(3), control.accept())
        .await
        .unwrap()
        .unwrap();
    let mut retired = 0;
    for socket in [&v4, &v6] {
        let mut first = None;
        let mut current = None;
        let mut bytes = [0; 4];
        for phase in 0..4u8 {
            let packet = tokio::time::timeout(
                Duration::from_secs(3),
                hps_net::transparent::recv_original(socket, &mut bytes),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!((packet.bytes, bytes[0]), (1, phase));
            let metadata = capture
                .routing_metadata(packet.source, packet.destination, 17)
                .unwrap();
            assert_eq!(metadata.context.process.as_ref().unwrap().pid, pid);
            assert_eq!(
                metadata.context.process.as_ref().unwrap().command,
                "hps-first-send"
            );
            assert_eq!(metadata.flow.revision, if phase < 2 { 7 } else { 8 });
            match phase {
                0 => {
                    first = Some(metadata.flow);
                    capture.activate(8, true).unwrap();
                }
                1 => {
                    let old = first.unwrap();
                    assert!(old.same_identity(&metadata.flow));
                    assert!(metadata.flow.last_seen_ns >= old.last_seen_ns);
                    let mut wrong = old;
                    wrong.revision = 8;
                    assert!(!capture.retire_flow(metadata.key, wrong).unwrap());
                    assert!(capture.retire_flow(metadata.key, old).unwrap());
                    retired += 1;
                    assert!(capture.flow(metadata.key).unwrap().is_none());
                }
                2 => {
                    assert!(!metadata.flow.same_identity(&first.unwrap()));
                    assert!(metadata.flow.first_seen_ns > first.unwrap().first_seen_ns);
                    assert!(!capture.retire_flow(metadata.key, first.unwrap()).unwrap());
                    current = Some(metadata.flow);
                }
                3 => {
                    assert!(metadata.flow.same_identity(&current.unwrap()));
                    capture.activate(7, true).unwrap();
                }
                _ => unreachable!(),
            }
            let reply = hps_net::transparent::bind_udp_reply(
                packet.destination,
                packet.source,
                specification.bypass_mark,
            )
            .unwrap();
            reply.send(&[phase]).await.unwrap();
            if phase == 2 {
                tokio::time::timeout(Duration::from_secs(3), control.read_exact(&mut bytes[..1]))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(bytes[0], b'X');
                assert!(
                    tokio::time::timeout(
                        Duration::from_millis(100),
                        hps_net::transparent::recv_original(socket, &mut bytes)
                    )
                    .await
                    .is_err()
                );
                assert!(
                    capture
                        .flow(metadata.key)
                        .unwrap()
                        .unwrap()
                        .same_identity(&metadata.flow)
                );
                control.write_all(b"Y").await.unwrap();
            }
            if phase == 3 {
                assert!(capture.retire_flow(metadata.key, metadata.flow).unwrap());
                retired += 1;
            }
        }
    }
    let mut bytes = [0; 4];
    let mut retained = vec![];
    for port in 38000..38064 {
        let packet = tokio::time::timeout(
            Duration::from_secs(3),
            hps_net::transparent::recv_original(&v4, &mut bytes),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            (packet.bytes, bytes[0], packet.destination.port()),
            (1, b'C', port)
        );
        let metadata = capture
            .routing_metadata(packet.source, packet.destination, 17)
            .unwrap();
        assert_eq!(metadata.flow.revision, 7);
        let reply = hps_net::transparent::bind_udp_reply(
            packet.destination,
            packet.source,
            specification.bypass_mark,
        )
        .unwrap();
        reply.send(b"C").await.unwrap();
        retained.push((metadata.key, metadata.flow));
    }
    tokio::time::timeout(Duration::from_secs(3), control.read_exact(&mut bytes[..1]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes[0], b'F');
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            hps_net::transparent::recv_original(&v4, &mut bytes)
        )
        .await
        .is_err()
    );
    assert_eq!(capture.flows().unwrap().len(), 64);
    for (key, flow) in &retained {
        assert!(capture.flow(*key).unwrap().unwrap().same_identity(flow));
    }
    let (key, flow) = retained.remove(0);
    assert!(capture.retire_flow(key, flow).unwrap());
    retired += 1;
    capture.activate(8, true).unwrap();
    control.write_all(b"G").await.unwrap();
    let packet = tokio::time::timeout(
        Duration::from_secs(3),
        hps_net::transparent::recv_original(&v4, &mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        (packet.bytes, bytes[0], packet.destination.port()),
        (1, b'H', 38064)
    );
    let metadata = capture
        .routing_metadata(packet.source, packet.destination, 17)
        .unwrap();
    assert_eq!(metadata.flow.revision, 8);
    let reply = hps_net::transparent::bind_udp_reply(
        packet.destination,
        packet.source,
        specification.bypass_mark,
    )
    .unwrap();
    reply.send(b"H").await.unwrap();
    retained.push((metadata.key, metadata.flow));
    let output = tokio::task::spawn_blocking(move || child.wait_with_output().unwrap())
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for (key, flow) in retained {
        assert!(capture.retire_flow(key, flow).unwrap());
        retired += 1;
    }
    assert!(capture.flows().unwrap().is_empty());
    let stats = capture.stats().unwrap();
    assert_eq!(stats.iter().map(|s| s.rejected_packets).sum::<u64>(), 3);
    assert_eq!(
        stats
            .iter()
            .map(|s| s.errors + s.missing_process)
            .sum::<u64>(),
        0
    );
    let fingerprint = capture.fingerprint.clone();
    capture.shutdown().unwrap();
    assert!(!capture.links_present().unwrap());
    assert!(!capture.routes_present().unwrap());
    println!(
        "{}",
        serde_json::json!({"scope":"kernel_flow_identity_only","kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":fingerprint,"abi_version":TP_ABI_VERSION,"ipv4":true,"ipv6":true,"verified_datagrams":73,"flow_capacity":64,"capacity_rejected_without_eviction":1,"caller_retry_after_exact_retirement":true,"old_revision":7,"new_revision":8,"exact_retirements":retired,"stale_retirement_preserved_new_flow":true,"conflicting_socket_cookies_rejected":2,"process_comm_latched_at_first_send":true,"full_transparent_datapath_ready":false})
    );
}
