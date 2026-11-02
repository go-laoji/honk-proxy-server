use super::{
    CaptureSockets, PreparedCapture,
    test_support::{Address, Group, Jobs, Membership},
};
use hps_config::{
    GroupConfig, GroupStrategy, LinuxCaptureConfig, ProcessCondition, ProxyConfig, ProxyProtocol,
    RuleConditions, RuleConfig, UpstreamConfig,
};
use hps_domain::{FlowOrigin, RouteAction, SelectionTransport};
use hps_engine::{
    capture::CaptureBudget,
    routing::RoutingCapabilities,
    runtime::Engine,
    udp::{decode_datagram, encode_datagram},
};
use std::{
    collections::HashMap,
    io::Write,
    net::SocketAddr,
    os::fd::AsFd,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

struct Proxy {
    address: SocketAddr,
    associations: Arc<AtomicUsize>,
    datagrams: Arc<AtomicUsize>,
    stop: CancellationToken,
    accept: Option<tokio::task::JoinHandle<()>>,
    workers: TaskTracker,
}
impl Proxy {
    async fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let associations = Arc::new(AtomicUsize::new(0));
        let datagrams = Arc::new(AtomicUsize::new(0));
        let stop = CancellationToken::new();
        let workers = TaskTracker::new();
        let cancel = stop.clone();
        let tracker = workers.clone();
        let connected = associations.clone();
        let relayed = datagrams.clone();
        let accept = tokio::spawn(async move {
            loop {
                let (control, peer) = tokio::select! {biased;_=cancel.cancelled()=>return,result=listener.accept()=>result.unwrap()};
                let cancel = cancel.clone();
                let connected = connected.clone();
                let relayed = relayed.clone();
                tracker.spawn(async move{tokio::select!{biased;_=cancel.cancelled()=>{},_=proxy_session(control,peer,connected,relayed)=>{}}});
            }
        });
        Self {
            address,
            associations,
            datagrams,
            stop,
            accept: Some(accept),
            workers,
        }
    }
    async fn shutdown(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.accept.take() {
            task.abort();
            let _ = task.await;
        }
        self.workers.close();
        tokio::time::timeout(Duration::from_secs(2), self.workers.wait())
            .await
            .unwrap();
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.accept {
            task.abort();
        }
    }
}
async fn proxy_session(
    mut control: TcpStream,
    peer: SocketAddr,
    connected: Arc<AtomicUsize>,
    relayed: Arc<AtomicUsize>,
) {
    let mut greeting = [0; 3];
    control.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 1, 0]);
    control.write_all(&[5, 0]).await.unwrap();
    let mut command = [0; 10];
    control.read_exact(&mut command).await.unwrap();
    assert_eq!(command, [5, 3, 0, 1, 0, 0, 0, 0, 0, 0]);
    let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut response = vec![5, 0, 0, 1, 127, 0, 0, 1];
    response.extend_from_slice(&relay.local_addr().unwrap().port().to_be_bytes());
    control.write_all(&response).await.unwrap();
    connected.fetch_add(1, Ordering::Relaxed);
    let mut packet = vec![0; 65536];
    let mut control_byte = [0; 1];
    loop {
        let (length, client) = tokio::select! {biased;_=control.read(&mut control_byte)=>return,result=relay.recv_from(&mut packet)=>result.unwrap()};
        assert_eq!(client.ip(), peer.ip());
        let (target, payload) = decode_datagram(&packet[..length]).unwrap();
        let destination = SocketAddr::new(target.host.parse().unwrap(), target.port);
        let outgoing = UdpSocket::bind(if destination.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .await
        .unwrap();
        outgoing.connect(destination).await.unwrap();
        outgoing.send(payload).await.unwrap();
        let mut reply = [0; 1024];
        let (length, source) = tokio::select! {biased;_=control.read(&mut control_byte)=>return,result=outgoing.recv_from(&mut reply)=>result.unwrap()};
        relayed.fetch_add(1, Ordering::Relaxed);
        let source =
            hps_engine::target::Target::new(&source.ip().to_string(), source.port()).unwrap();
        relay
            .send_to(&encode_datagram(&source, &reply[..length]).unwrap(), client)
            .await
            .unwrap();
    }
}

#[test]
#[ignore = "helper invoked only by isolated real_transparent_udp_shared_engine"]
fn transparent_udp_child() {
    let group = std::env::var("HPS_TRANSPARENT_UDP_CGROUP").unwrap();
    let group = Path::new(&group).canonicalize().unwrap();
    assert!(
        group
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hps-transparent-udp-")
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(group.join("cgroup.procs"))
        .unwrap()
        .write_all(format!("{}\n", std::process::id()).as_bytes())
        .unwrap();
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_NAME, c"hps-udp-client".as_ptr(), 0, 0, 0) },
        0
    );
    let targets: Vec<SocketAddr> =
        serde_json::from_str(&std::env::var("HPS_TRANSPARENT_UDP_TARGETS").unwrap()).unwrap();
    assert_eq!(targets.len(), 4);
    let mut held = vec![];
    for ipv4 in [true, false] {
        for connected in [false, true] {
            let socket =
                std::net::UdpSocket::bind(if ipv4 { "0.0.0.0:0" } else { "[::]:0" }).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            for target in targets.iter().copied().filter(|t| t.is_ipv4() == ipv4) {
                if connected {
                    socket.connect(target).unwrap();
                    assert_eq!(socket.peer_addr().unwrap(), target);
                    socket.send(b"kernel-udp-forward").unwrap();
                } else {
                    socket.send_to(b"kernel-udp-forward", target).unwrap();
                }
                let mut reply = [0; 64];
                let (length, source) = socket.recv_from(&mut reply).unwrap();
                assert_eq!(source, target);
                assert_eq!(&reply[..length], b"kernel-udp-reply");
            }
            held.push(socket);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "requires isolated privileged Linux, private writable cgroup v2 and actual eBPF object; real dual-stack UDP targets and SOCKS5 relay"]
async fn real_transparent_udp_shared_engine() {
    let _addresses = [Address::add("203.0.113.77"), Address::add("2001:db8::77")];
    for via_proxy in [false, true] {
        let root = std::env::var("HPS_TEST_CGROUP_ROOT").unwrap();
        let group = Group(Path::new(&root).join(format!(
            "hps-transparent-udp-{}-{}",
            std::process::id(),
            u8::from(via_proxy)
        )));
        std::fs::create_dir(&group.0).unwrap();
        let specification = LinuxCaptureConfig {
            object: std::env::var("HPS_EBPF_OBJECT").unwrap().into(),
            local_cgroup: Some(group.0.clone()),
            tcp: false,
            udp_port: 0,
            udp_idle_secs: 1,
            max_pending: 16,
            max_udp_sessions: 16,
            ..Default::default()
        };
        let mut origins = Jobs(vec![]);
        let mut targets = vec![];
        for ip in ["203.0.113.77", "2001:db8::77"] {
            for _ in 0..2 {
                // Reuse permits the per-client connected transparent reply FDs
                // to bind the real target port in this isolated fixture netns.
                let origin = hps_net::transparent::bind_udp(
                    SocketAddr::new(ip.parse().unwrap(), 0),
                    specification.bypass_mark,
                )
                .unwrap();
                targets.push(origin.local_addr().unwrap());
                origins.0.push(tokio::spawn(async move {
                    let mut bytes = [0; 64];
                    for _ in 0..2 {
                        let (length, peer) = origin.recv_from(&mut bytes).await.unwrap();
                        assert_eq!(&bytes[..length], b"kernel-udp-forward");
                        origin.send_to(b"kernel-udp-reply", peer).await.unwrap();
                    }
                }));
            }
        }
        let mut proxy = Proxy::bind().await;
        let mut config = ProxyConfig::default();
        config.routing.default_action = RouteAction::Reject;
        let action = if via_proxy {
            config.upstreams.push(UpstreamConfig {
                id: "udp-socks".into(),
                protocol: ProxyProtocol::Socks5,
                host: proxy.address.ip().to_string(),
                port: proxy.address.port(),
                enabled: true,
                credentials: None,
                expires_at_unix: None,
                ca_certificate: None,
                tls_server_name: None,
            });
            config.selection.groups.push(GroupConfig {
                id: "udp-egress".into(),
                strategy: GroupStrategy::Fixed,
                members: vec!["udp-socks".into()],
                pinned_node: Some("udp-socks".into()),
                ..Default::default()
            });
            RouteAction::Group {
                id: "udp-egress".into(),
            }
        } else {
            RouteAction::Direct
        };
        config.routing.rules.push(RuleConfig {
            id: "verified-udp".into(),
            enabled: true,
            conditions: RuleConditions {
                transports: vec![SelectionTransport::Udp],
                process: Some(ProcessCondition {
                    command: Some("hps-udp-client".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            action,
        });
        let udp4 =
            hps_net::transparent::bind_udp("0.0.0.0:0".parse().unwrap(), specification.bypass_mark)
                .unwrap();
        let udp6 =
            hps_net::transparent::bind_udp("[::]:0".parse().unwrap(), specification.bypass_mark)
                .unwrap();
        let capture = Arc::new(
            PreparedCapture::read(&specification)
                .unwrap()
                .load(CaptureSockets {
                    tcp4: None,
                    tcp6: None,
                    udp4: Some(udp4.as_fd()),
                    udp6: Some(udp6.as_fd()),
                })
                .unwrap(),
        );
        capture.prepare_routes().unwrap();
        let engine = Engine::with_capture(
            config,
            None,
            RoutingCapabilities {
                local_process: true,
                udp: true,
            },
        )
        .unwrap();
        engine.set_generation(7);
        let budget = CaptureBudget::new(&specification).unwrap();
        let scope = CancellationToken::new();
        let mut receiving = vec![];
        for socket in [udp4, udp6] {
            let capture = capture.clone();
            let engine = engine.clone();
            let budget = budget.clone();
            let scope = scope.clone();
            let mark = specification.bypass_mark;
            receiving.push(tokio::spawn(async move {
                let mut sessions = HashMap::new();
                let mut pids = vec![];
                let mut bytes = [0; 1024];
                for _ in 0..4 {
                    let packet = hps_net::transparent::recv_original(&socket, &mut bytes)
                        .await
                        .unwrap();
                    let (revision, context) = capture
                        .routing_context(packet.source, packet.destination, 17)
                        .unwrap();
                    assert_eq!(context.origin, FlowOrigin::LinuxLocal);
                    let process = context.process.as_ref().unwrap();
                    assert_eq!(process.command, "hps-udp-client");
                    pids.push(process.pid);
                    let key = (packet.source, packet.destination);
                    let session = sessions.entry(key).or_insert_with(|| {
                        let reply = hps_net::transparent::bind_udp_reply(
                            packet.destination,
                            packet.source,
                            mark,
                        )
                        .unwrap();
                        engine
                            .spawn_captured_udp(
                                reply,
                                context,
                                revision,
                                budget.udp_admission(&scope).unwrap(),
                            )
                            .unwrap()
                    });
                    session.try_send(&bytes[..packet.bytes]).unwrap();
                }
                (sessions, pids)
            }));
        }
        // All daemon threads join the selected cgroup. Direct/relay outgoing
        // sockets are created after activation, exercising the real TGID bypass.
        let membership = Membership::enter(&group.0);
        capture.activate(7, true).unwrap();
        let exe = std::env::current_exe().unwrap();
        let cgroup = group.0.clone();
        let destinations = serde_json::to_string(&targets).unwrap();
        let (pid, output) = tokio::task::spawn_blocking(move || {
            let child = Command::new(exe)
                .args([
                    "--exact",
                    "transparent::udp_tests::transparent_udp_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("HPS_TRANSPARENT_UDP_CGROUP", cgroup)
                .env("HPS_TRANSPARENT_UDP_TARGETS", destinations)
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
            "{}\n{}\nproxy={via_proxy}; kernel_stats={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            capture
                .stats()
                .unwrap()
                .iter()
                .map(|s| (
                    s.captured_packets,
                    s.missing_process,
                    s.errors,
                    s.last_error,
                    s.error_phase
                ))
                .collect::<Vec<_>>()
        );
        let mut held = vec![];
        for receiver in receiving {
            let (sessions, pids) = tokio::time::timeout(Duration::from_secs(2), receiver)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(pids, &[pid; 4]);
            held.extend(sessions.into_values());
        }
        for origin in &mut origins.0 {
            tokio::time::timeout(Duration::from_secs(2), origin)
                .await
                .unwrap()
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(4), engine.wait_captured_idle())
            .await
            .unwrap();
        assert!(held.iter().all(|s| s.is_closed()));
        let usage = budget.usage();
        assert_eq!(
            (
                usage.pending,
                usage.udp_sessions,
                usage.queued_datagrams,
                usage.queued_bytes
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(usage.closed_udp_sessions, 8);
        assert_eq!(usage.dropped_datagrams, 0);
        assert_eq!(usage.invalid_replies, 0);
        let (stats, flows) = engine.node_registry().connection_records(true);
        assert_eq!(stats.opened, 8);
        assert_eq!(stats.failed, 0);
        assert_eq!(stats.bytes_up, 144);
        assert_eq!(stats.bytes_down, 128);
        assert!(
            flows
                .iter()
                .all(|f| f.context.origin == FlowOrigin::LinuxLocal
                    && f.context.transport == SelectionTransport::Udp
                    && f.context.process.as_ref().unwrap().pid == pid
                    && f.routing.configuration_revision == 7
                    && f.routing.rule_id.as_deref() == Some("verified-udp"))
        );
        if via_proxy {
            assert_eq!(proxy.associations.load(Ordering::Relaxed), 8);
            assert_eq!(proxy.datagrams.load(Ordering::Relaxed), 8);
            assert!(
                flows
                    .iter()
                    .all(|f| f.choice.as_ref().unwrap().node_id == "udp-socks"
                        && f.selection.as_ref().unwrap().group.as_deref() == Some("udp-egress"))
            );
        } else {
            assert_eq!(proxy.associations.load(Ordering::Relaxed), 0);
            assert!(engine.demand_age().is_none());
            assert!(
                flows
                    .iter()
                    .all(|f| f.choice.is_none() && f.routing.action == RouteAction::Direct)
            );
        }
        assert!(
            capture
                .stats()
                .unwrap()
                .iter()
                .all(|s| s.errors == 0 && s.missing_process == 0)
        );
        scope.cancel();
        capture.shutdown().unwrap();
        engine.shutdown().await;
        proxy.shutdown().await;
        drop(membership);
        assert!(!capture.routes_present().unwrap() && !capture.links_present().unwrap());
        println!(
            "{}",
            serde_json::json!({"scope":"local_udp_shared_engine_only","mode":if via_proxy{"socks5"}else{"direct"},"kernel":std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap().trim(),"object_sha256":capture.fingerprint,"revision":7,"verified_flows":8,"client_pid":pid,"ipv4":true,"ipv6":true,"connected_and_unconnected_udp":true,"original_reply_source":true,"daemon_in_selected_cgroup_bypassed":true,"process_rule":true,"business_bytes_up":stats.bytes_up,"business_bytes_down":stats.bytes_down,"idle_sessions_released":usage.closed_udp_sessions,"forwarding_verified":true,"full_transparent_datapath_ready":false})
        );
    }
}
