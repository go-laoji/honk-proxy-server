use aya_ebpf::{
    EbpfContext,
    bindings::{__sk_buff, BPF_F_CURRENT_NETNS, bpf_pidns_info, bpf_sock_tuple},
    helpers::{
        bpf_get_current_cgroup_id, bpf_get_current_comm, bpf_get_netns_cookie,
        bpf_get_ns_current_pid_tgid, bpf_get_socket_cookie, bpf_getsockopt, bpf_ktime_get_ns,
        bpf_map_lookup_elem, bpf_setsockopt, bpf_sk_assign, bpf_sk_release, bpf_skc_lookup_tcp,
    },
    macros::{cgroup_sock, cgroup_sock_addr, classifier, map},
    maps::{Array, HashMap, LpmTrie, PerCpuArray, SockMap, lpm_trie::Key},
    programs::{SockAddrContext, SockContext, TcContext},
};
use hps_ebpf_abi::{
    CapturePolicy, CaptureStats, CapturedFlow, FlowKey, SocketProcess, TP_ABI_VERSION, TP_ENABLED,
    TP_FLOW_CAPACITY, TP_GATEWAY, TP_IPV4, TP_IPV6, TP_LOCAL, TP_MAX_BYPASS, TP_MAX_INTERFACES,
    TP_PROCESS_VERIFIED, TP_REQUIRE_PROCESS, TP_SOCKET_COUNT, TP_SOCKET_TCP4, TP_SOCKET_TCP6,
    TP_SOCKET_UDP4, TP_SOCKET_UDP6, TP_SUSPENDED, TP_TCP, TP_UDP,
    packet::{self, PacketHeader, PacketRead},
};

const NEXT: i32 = -1;
const PASS: i32 = 0;
const DROP: i32 = 2;
const SOL_SOCKET: i32 = 1;
const SO_MARK: i32 = 36;

#[map]
// NO_PREALLOC hash updates publish a new immutable element under RCU. An Array
// update copies into the live value and can expose a mixed policy to a hook.
static HPS_TP_POLICY: HashMap<u32, CapturePolicy> = HashMap::with_max_entries(1, 1);
#[map]
static HPS_TP_STATS: PerCpuArray<CaptureStats> = PerCpuArray::with_max_entries(3, 0);
#[map]
static HPS_TP_ABI4: Array<u32> = Array::with_max_entries(1, 128); // BPF_F_RDONLY_PROG
#[map]
static HPS_TP_PROCESS: HashMap<u64, SocketProcess> = HashMap::with_max_entries(TP_FLOW_CAPACITY, 1);
#[map]
static HPS_TP_MODES: HashMap<u64, u32> = HashMap::with_max_entries(TP_FLOW_CAPACITY, 0);
#[map]
// Identity and generation are insert-only. Only the owning user-space runtime
// retires exact identities; capacity exhaustion drops rather than evicts a flow.
static HPS_TP_FLOWS: HashMap<FlowKey, CapturedFlow> =
    HashMap::with_max_entries(TP_FLOW_CAPACITY, 1);
#[map]
static HPS_TP_LAN: HashMap<u32, u8> = HashMap::with_max_entries(TP_MAX_INTERFACES, 0);
#[map]
static HPS_TP_BYPASS4: LpmTrie<u32, u8> = LpmTrie::with_max_entries(TP_MAX_BYPASS, 0);
#[map]
static HPS_TP_BYPASS6: LpmTrie<[u32; 4], u8> = LpmTrie::with_max_entries(TP_MAX_BYPASS, 0);
#[map]
static HPS_TP_PORTS: HashMap<u32, u8> = HashMap::with_max_entries(TP_MAX_BYPASS, 0);
#[map]
static HPS_TP_SOCKETS: SockMap = SockMap::with_max_entries(TP_SOCKET_COUNT, 0);

#[inline(always)]
fn policy() -> Option<CapturePolicy> {
    if HPS_TP_ABI4.get(0).copied() != Some(TP_ABI_VERSION) {
        return None;
    }
    let policy = *unsafe { HPS_TP_POLICY.get(&0) }?;
    if policy.abi_version != TP_ABI_VERSION
        || policy.flags & TP_ENABLED == 0
        || policy.revision == 0
    {
        return None;
    }
    Some(policy)
}
#[inline(always)]
fn socket_policy(ctx: &SockAddrContext) -> Option<CapturePolicy> {
    let policy = policy()?;
    if unsafe { bpf_get_netns_cookie(ctx.as_ptr()) } != policy.netns_cookie {
        return None;
    }
    Some(policy)
}
#[inline(always)]
fn enabled(policy: &CapturePolicy, family: u32, protocol: u32) -> bool {
    (family == 4 && policy.flags & TP_IPV4 != 0 || family == 6 && policy.flags & TP_IPV6 != 0)
        && (protocol == 6 && policy.flags & TP_TCP != 0
            || protocol == 17 && policy.flags & TP_UDP != 0)
}
#[inline(always)]
fn bypass(family: u32, address: [u32; 4]) -> bool {
    if family == 4 {
        HPS_TP_BYPASS4.get(&Key::new(32, address[0])).is_some()
    } else {
        HPS_TP_BYPASS6.get(&Key::new(128, address)).is_some()
    }
}
#[inline(always)]
fn excluded_port(port: u32) -> bool {
    unsafe { HPS_TP_PORTS.get(&port) }.is_some()
}
#[cgroup_sock_addr(connect4)]
pub fn hps_tp_connect4(ctx: SockAddrContext) -> i32 {
    mark_socket(ctx, 4, false)
}
#[cgroup_sock_addr(connect6)]
pub fn hps_tp_connect6(ctx: SockAddrContext) -> i32 {
    mark_socket(ctx, 6, false)
}
#[cgroup_sock_addr(sendmsg4)]
pub fn hps_tp_send4(ctx: SockAddrContext) -> i32 {
    mark_socket(ctx, 4, true)
}
#[cgroup_sock_addr(sendmsg6)]
pub fn hps_tp_send6(ctx: SockAddrContext) -> i32 {
    mark_socket(ctx, 6, true)
}
#[cgroup_sock(sock_release)]
pub fn hps_tp_release(ctx: SockContext) -> i32 {
    let cookie = unsafe { bpf_get_socket_cookie(ctx.as_ptr()) };
    let _ = HPS_TP_MODES.remove(&cookie);
    let _ = HPS_TP_PROCESS.remove(&cookie);
    1
}
#[cgroup_sock(sock_create)]
pub fn hps_tp_create(ctx: SockContext) -> i32 {
    let Some(policy) = policy() else {
        return 1;
    };
    if unsafe { bpf_get_netns_cookie(ctx.as_ptr()) } != policy.netns_cookie {
        return 1;
    }
    let raw = unsafe { &mut *ctx.sock };
    let family = match raw.family {
        2 => 4,
        10 => 6,
        _ => return 1,
    };
    if raw.protocol != 17 || !enabled(&policy, family, 17) || raw.mark != 0 {
        return 1;
    }
    let mut identity = bpf_pidns_info { pid: 0, tgid: 0 };
    let known = unsafe {
        bpf_get_ns_current_pid_tgid(
            policy.pidns_device,
            policy.pidns_inode,
            &mut identity,
            core::mem::size_of::<bpf_pidns_info>() as u32,
        )
    };
    if known != 0 {
        return 0;
    }
    if identity.tgid != policy.excluded_tgid {
        raw.mark = policy.capture_mark;
    }
    1
}

#[inline(always)]
fn mark_socket(ctx: SockAddrContext, family: u32, sendmsg: bool) -> i32 {
    let Some(policy) = socket_policy(&ctx) else {
        return 1;
    };
    // SAFETY: kernel supplies the matching cgroup sock_addr context for each hook.
    let raw = unsafe { &*ctx.sock_addr };
    let port = u32::from(u16::from_be(raw.user_port as u16));
    let destination = if family == 4 {
        [raw.user_ip4, 0, 0, 0]
    } else {
        // Preserve fixed context offsets; LLVM memcpy of a context array may
        // introduce a modified ctx pointer that the kernel correctly rejects.
        unsafe {
            [
                core::ptr::read_volatile(core::ptr::addr_of!(raw.user_ip6[0])),
                core::ptr::read_volatile(core::ptr::addr_of!(raw.user_ip6[1])),
                core::ptr::read_volatile(core::ptr::addr_of!(raw.user_ip6[2])),
                core::ptr::read_volatile(core::ptr::addr_of!(raw.user_ip6[3])),
            ]
        }
    };
    if !enabled(&policy, family, raw.protocol) {
        return 1;
    }
    let bypassed = bypass(family, destination) || excluded_port(port);
    let Some(stats) = HPS_TP_STATS.get_ptr_mut(0) else {
        return 0;
    };
    // SAFETY: non-sleepable hook and fixed per-CPU map value, no pointer escapes.
    let stats = unsafe { &mut *stats };
    let mut identity = bpf_pidns_info { pid: 0, tgid: 0 };
    let known = unsafe {
        bpf_get_ns_current_pid_tgid(
            policy.pidns_device,
            policy.pidns_inode,
            &mut identity,
            core::mem::size_of::<bpf_pidns_info>() as u32,
        )
    };
    if known != 0 {
        stats.errors = stats.errors.wrapping_add(1);
        stats.last_error = known;
        stats.error_phase = 1;
        return 0;
    }
    if identity.tgid == policy.excluded_tgid {
        return 1;
    }
    let mut mark = 0u32;
    let read = unsafe {
        bpf_getsockopt(
            ctx.as_ptr(),
            SOL_SOCKET,
            SO_MARK,
            core::ptr::addr_of_mut!(mark).cast(),
            4,
        )
    };
    if read != 0 {
        stats.errors = stats.errors.wrapping_add(1);
        stats.last_error = read;
        stats.error_phase = 2;
        return 0;
    }
    // Existing foreign marks and our explicitly marked daemon replies are preserved.
    if mark != 0 && mark != policy.capture_mark {
        return 1;
    }
    if !bypassed && policy.flags & TP_SUSPENDED != 0 {
        stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
        stats.error_phase = 14;
        return 0;
    }
    let cookie = unsafe { bpf_get_socket_cookie(ctx.as_ptr()) };
    if cookie == 0 {
        stats.errors = stats.errors.wrapping_add(1);
        stats.error_phase = 3;
        return 0;
    }
    if raw.protocol == 17 {
        // SO_MARK belongs to the UDP socket, not to one sendmsg. Keep its
        // capture/bypass class fixed until close, and fail conflicting mixed
        // destinations instead of racing concurrent sends into the wrong route.
        let desired = if bypassed { 2u32 } else { 1u32 };
        let mut mode = unsafe { HPS_TP_MODES.get(&cookie) }.copied();
        if mode.is_none() {
            let _ = HPS_TP_MODES.insert(&cookie, &desired, 1);
            mode = unsafe { HPS_TP_MODES.get(&cookie) }.copied();
        }
        if mode != Some(desired) {
            stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
            stats.unsupported_packets = stats.unsupported_packets.wrapping_add(1);
            stats.error_phase = 8;
            return 0;
        }
        // sendmsg snapshots the routing mark before this hook on both families.
        // A pre-existing unconnected socket must fail this first send rather
        // than leak it to the main route. Newly created sockets are initialized
        // by sock_create; setting the mark here permits a later caller retry.
        if sendmsg && !bypassed && mark == 0 {
            mark = policy.capture_mark;
            let _ = unsafe {
                bpf_setsockopt(
                    ctx.as_ptr(),
                    SOL_SOCKET,
                    SO_MARK,
                    core::ptr::addr_of_mut!(mark).cast(),
                    4,
                )
            };
            stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
            stats.error_phase = 9;
            return 0;
        }
    }
    if bypassed {
        if mark == policy.capture_mark {
            mark = 0;
            if unsafe {
                bpf_setsockopt(
                    ctx.as_ptr(),
                    SOL_SOCKET,
                    SO_MARK,
                    core::ptr::addr_of_mut!(mark).cast(),
                    4,
                )
            } != 0
            {
                return 0;
            }
        }
        return 1;
    }
    let process = SocketProcess {
        socket_cookie: cookie,
        cgroup_id: unsafe { bpf_get_current_cgroup_id() },
        observed_at_ns: unsafe { bpf_ktime_get_ns() },
        pid: identity.tgid,
        thread_id: identity.pid,
        command: bpf_get_current_comm().unwrap_or([0; 16]),
    };
    if let Err(error) = HPS_TP_PROCESS.insert(&cookie, &process, 1) {
        let same_owner = unsafe { HPS_TP_PROCESS.get(&cookie) }
            .is_some_and(|old| old.socket_cookie == cookie && old.pid == identity.tgid);
        if !same_owner {
            stats.errors = stats.errors.wrapping_add(1);
            stats.last_error = i64::from(error);
            stats.error_phase = 4;
            return 0;
        }
    }
    stats.observed_sockets = stats.observed_sockets.wrapping_add(1);
    mark = policy.capture_mark;
    let result = unsafe {
        bpf_setsockopt(
            ctx.as_ptr(),
            SOL_SOCKET,
            SO_MARK,
            core::ptr::addr_of_mut!(mark).cast(),
            4,
        )
    };
    if result != 0 {
        stats.errors = stats.errors.wrapping_add(1);
        stats.last_error = result;
        stats.error_phase = 5;
        return 0;
    }
    stats.marked_sockets = stats.marked_sockets.wrapping_add(1);
    1
}
struct Packet<'a>(&'a TcContext);
impl PacketRead for Packet<'_> {
    #[inline(always)]
    fn packet_len(&self) -> usize {
        self.0.len() as usize
    }
    #[inline(always)]
    fn load<const N: usize>(&self, offset: usize) -> Option<[u8; N]> {
        self.0.load(offset).ok()
    }
}
#[inline(always)]
fn key(packet: &PacketHeader, policy: &CapturePolicy) -> FlowKey {
    FlowKey {
        family: packet.family,
        protocol: packet.protocol,
        source: packet.source,
        destination: packet.destination,
        source_port: u32::from(packet.source_port),
        destination_port: u32::from(packet.destination_port),
        netns_cookie: policy.netns_cookie,
    }
}
#[inline(always)]
fn matches_packet(flow: &CapturedFlow, packet: &PacketHeader) -> bool {
    packet.protocol != 6
        || packet.tcp_flags & 2 == 0
        || flow.initial_sequence == packet.tcp_sequence
}
#[inline(always)]
fn touch(pointer: *mut CapturedFlow, now: u64) {
    // The NO_PREALLOC hash keeps this node alive under the current BPF RCU
    // read section. Never reinsert an old whole-value copy after another CPU's
    // retirement. This timestamp is an approximate activity hint, not identity.
    unsafe { core::ptr::write_volatile(core::ptr::addr_of_mut!((*pointer).last_seen_ns), now) };
}
#[classifier]
pub fn hps_tp_egress(ctx: TcContext) -> i32 {
    // TCX links are created in the loader's network namespace. Linux 6.12 does
    // not allow get_netns_cookie in TC; use that link-bound namespace identity.
    let Some(policy) = policy() else {
        return NEXT;
    };
    let raw = unsafe { &*ctx.as_ptr().cast::<__sk_buff>() };
    if raw.mark != policy.capture_mark {
        return NEXT;
    }
    let Ok(packet) = packet::parse(&Packet(&ctx)) else {
        return NEXT;
    };
    if !enabled(&policy, packet.family, packet.protocol)
        || bypass(packet.family, packet.destination)
        || excluded_port(u32::from(packet.destination_port))
    {
        return NEXT;
    }
    if policy.flags & TP_SUSPENDED != 0 {
        return reject_egress(-1, 14);
    }
    let cookie = unsafe { bpf_get_socket_cookie(ctx.as_ptr()) };
    let Some(process) = unsafe { HPS_TP_PROCESS.get(&cookie) }.copied() else {
        return NEXT;
    };
    let key = key(&packet, &policy);
    let now = unsafe { bpf_ktime_get_ns() };
    if let Some(pointer) = HPS_TP_FLOWS.get_ptr_mut(&key) {
        let flow = unsafe { &*pointer };
        if flow.origin == TP_LOCAL
            && flow.process.socket_cookie == cookie
            && matches_packet(flow, &packet)
        {
            touch(pointer, now);
            return NEXT;
        }
        return reject_egress(-17, 10);
    }
    if packet.protocol == 6 && packet.tcp_flags & 0x12 != 2 {
        return reject_egress(-2, 11);
    }
    let flow = CapturedFlow {
        revision: policy.revision,
        first_seen_ns: now,
        last_seen_ns: now,
        process,
        origin: TP_LOCAL,
        flags: TP_PROCESS_VERIFIED,
        initial_sequence: packet.tcp_sequence,
        reserved: 0,
    };
    if let Err(error) = HPS_TP_FLOWS.insert(&key, &flow, 1) {
        // Concurrent first packets may choose either complete policy, but only
        // the first successful insertion owns this tuple until exact retirement.
        if let Some(pointer) = HPS_TP_FLOWS.get_ptr_mut(&key) {
            let old = unsafe { &*pointer };
            if old.origin == TP_LOCAL
                && old.process.socket_cookie == cookie
                && matches_packet(old, &packet)
            {
                touch(pointer, now);
                return NEXT;
            }
        }
        return reject_egress(i64::from(error), 12);
    }
    NEXT
}
#[inline(always)]
fn reject_egress(error: i64, phase: u64) -> i32 {
    if let Some(pointer) = HPS_TP_STATS.get_ptr_mut(1) {
        let stats = unsafe { &mut *pointer };
        stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
        stats.last_error = error;
        stats.error_phase = phase;
    }
    DROP
}

#[classifier]
pub fn hps_tp_ingress(ctx: TcContext) -> i32 {
    let Some(policy) = policy() else {
        return NEXT;
    };
    let raw = unsafe { &*ctx.as_ptr().cast::<__sk_buff>() };
    // Helper map keys must be stack/map memory, never an address into ctx.
    let ifindex = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(raw.ifindex)) };
    let lan = unsafe { HPS_TP_LAN.get(&ifindex) }.is_some();
    let local = !lan && raw.mark == policy.capture_mark;
    if (!local && (!lan || raw.mark != 0)) || raw.pkt_type != 0 {
        return NEXT;
    }
    let Some(stats) = HPS_TP_STATS.get_ptr_mut(2) else {
        return DROP;
    };
    let stats = unsafe { &mut *stats };
    let network = match packet::network_header(&Packet(&ctx)) {
        Ok(header) => header,
        Err(packet::PacketError::NotIp) => return NEXT,
        Err(_) => {
            stats.unsupported_packets = stats.unsupported_packets.wrapping_add(1);
            return DROP;
        }
    };
    if bypass(network.family, network.destination) {
        stats.bypassed_packets = stats.bypassed_packets.wrapping_add(1);
        return NEXT;
    }
    let packet = match packet::transport_header(&Packet(&ctx), &network) {
        Ok(packet) => packet,
        Err(packet::PacketError::NotTransport) => return NEXT,
        Err(_) => {
            stats.unsupported_packets = stats.unsupported_packets.wrapping_add(1);
            return DROP;
        }
    };
    if !enabled(&policy, packet.family, packet.protocol)
        || excluded_port(u32::from(packet.destination_port))
    {
        stats.bypassed_packets = stats.bypassed_packets.wrapping_add(1);
        return NEXT;
    }
    let key = key(&packet, &policy);
    let existing = unsafe { HPS_TP_FLOWS.get(&key) }.copied();
    if policy.flags & TP_SUSPENDED != 0 {
        stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
        stats.error_phase = 14;
        return DROP;
    }
    if existing.is_some_and(|flow| {
        flow.origin != if local { TP_LOCAL } else { TP_GATEWAY } || !matches_packet(&flow, &packet)
    }) {
        stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
        stats.error_phase = 13;
        return DROP;
    }
    if local
        && policy.flags & TP_REQUIRE_PROCESS != 0
        && !existing
            .is_some_and(|flow| flow.origin == TP_LOCAL && flow.flags & TP_PROCESS_VERIFIED != 0)
    {
        stats.missing_process = stats.missing_process.wrapping_add(1);
        stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
        return DROP;
    }
    if existing.is_none() {
        if packet.protocol == 6 && packet.tcp_flags & 0x12 != 2 {
            stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
            return DROP;
        }
        let now = unsafe { bpf_ktime_get_ns() };
        let flow = CapturedFlow {
            revision: policy.revision,
            first_seen_ns: now,
            last_seen_ns: now,
            process: SocketProcess::default(),
            origin: if local { TP_LOCAL } else { TP_GATEWAY },
            flags: 0,
            initial_sequence: packet.tcp_sequence,
            reserved: 0,
        };
        if let Err(error) = HPS_TP_FLOWS.insert(&key, &flow, 1) {
            // A concurrent first packet may have won the same key.
            if !unsafe { HPS_TP_FLOWS.get(&key) }
                .is_some_and(|old| old.origin == flow.origin && matches_packet(old, &packet))
            {
                stats.errors = stats.errors.wrapping_add(1);
                stats.last_error = i64::from(error);
                stats.error_phase = 6;
                return DROP;
            }
        }
    }
    if let Some(pointer) = HPS_TP_FLOWS.get_ptr_mut(&key) {
        touch(pointer, unsafe { bpf_ktime_get_ns() });
    }
    ctx.set_mark(policy.capture_mark);
    let result = if packet.protocol == 6 {
        assign_tcp(&ctx, &packet)
    } else {
        assign_listener(
            &ctx,
            if packet.family == 4 {
                TP_SOCKET_UDP4
            } else {
                TP_SOCKET_UDP6
            },
        )
    };
    if result != 0 {
        stats.errors = stats.errors.wrapping_add(1);
        stats.last_error = result;
        stats.error_phase = 7;
        stats.rejected_packets = stats.rejected_packets.wrapping_add(1);
        DROP
    } else {
        stats.captured_packets = stats.captured_packets.wrapping_add(1);
        PASS
    }
}
#[inline(always)]
fn assign_listener(ctx: &TcContext, index: u32) -> i64 {
    // SockMap is repr(transparent) over its map definition in locked Aya 0.3.
    // Taking its symbol address lets the loader relocate this ordinary map helper.
    let socket = unsafe {
        bpf_map_lookup_elem(
            core::ptr::addr_of!(HPS_TP_SOCKETS).cast_mut().cast(),
            core::ptr::addr_of!(index).cast(),
        )
    };
    if socket.is_null() {
        return -2;
    }
    let result = unsafe { bpf_sk_assign(ctx.as_ptr(), socket, 0) };
    let _ = unsafe { bpf_sk_release(socket) };
    result
}
#[inline(always)]
fn assign_tcp(ctx: &TcContext, packet: &PacketHeader) -> i64 {
    let mut tuple: bpf_sock_tuple = unsafe { core::mem::zeroed() };
    let size = if packet.family == 4 {
        tuple.__bindgen_anon_1.ipv4.saddr = packet.source[0];
        tuple.__bindgen_anon_1.ipv4.daddr = packet.destination[0];
        tuple.__bindgen_anon_1.ipv4.sport = packet.source_port.to_be();
        tuple.__bindgen_anon_1.ipv4.dport = packet.destination_port.to_be();
        12
    } else {
        tuple.__bindgen_anon_1.ipv6.saddr = packet.source;
        tuple.__bindgen_anon_1.ipv6.daddr = packet.destination;
        tuple.__bindgen_anon_1.ipv6.sport = packet.source_port.to_be();
        tuple.__bindgen_anon_1.ipv6.dport = packet.destination_port.to_be();
        36
    };
    let socket = unsafe {
        bpf_skc_lookup_tcp(
            ctx.as_ptr(),
            &mut tuple,
            size,
            BPF_F_CURRENT_NETNS as u64,
            0,
        )
    };
    if !socket.is_null() {
        // TCP_LISTEN == 10. Existing transparent children keep their actual socket.
        if unsafe { (*socket).state } != 10 {
            let result = unsafe { bpf_sk_assign(ctx.as_ptr(), socket.cast(), 0) };
            let _ = unsafe { bpf_sk_release(socket.cast()) };
            return result;
        }
        let _ = unsafe { bpf_sk_release(socket.cast()) };
    }
    assign_listener(
        ctx,
        if packet.family == 4 {
            TP_SOCKET_TCP4
        } else {
            TP_SOCKET_TCP6
        },
    )
}
