use aya_ebpf::{
    EbpfContext,
    helpers::{bpf_get_current_pid_tgid, bpf_get_netns_cookie},
    macros::{cgroup_sock_addr, map},
    maps::{Array, HashMap, PerCpuArray},
    programs::SockAddrContext,
};
use core::intrinsics::{AtomicOrdering, atomic_xadd};
use hps_ebpf_abi::{
    DNS_ABI_VERSION, DNS_ENABLED, DNS_PEER_CAPACITY, DnsAllocation, DnsPeer, DnsPolicy, DnsStats,
};

#[map]
static HPS_DNS_POLICY: Array<DnsPolicy> = Array::with_max_entries(1, 0);
#[map]
static HPS_DNS_ALLOC: Array<DnsAllocation> = Array::with_max_entries(1, 0);
#[map]
static HPS_DNS_PEERS: HashMap<DnsPeer, u32> = HashMap::with_max_entries(DNS_PEER_CAPACITY, 0);
#[map]
static HPS_DNS_REVERSE: HashMap<u32, DnsPeer> = HashMap::with_max_entries(DNS_PEER_CAPACITY, 0);
#[map]
static HPS_DNS_STATS: PerCpuArray<DnsStats> = PerCpuArray::with_max_entries(2, 0);

#[cgroup_sock_addr(connect4)]
pub fn hps_dns_connect4(ctx: SockAddrContext) -> i32 {
    redirect(ctx, 4)
}
#[cgroup_sock_addr(connect6)]
pub fn hps_dns_connect6(ctx: SockAddrContext) -> i32 {
    redirect(ctx, 6)
}
#[cgroup_sock_addr(sendmsg4)]
pub fn hps_dns_send4(ctx: SockAddrContext) -> i32 {
    redirect(ctx, 4)
}
#[cgroup_sock_addr(sendmsg6)]
pub fn hps_dns_send6(ctx: SockAddrContext) -> i32 {
    redirect(ctx, 6)
}
#[cgroup_sock_addr(recvmsg4)]
pub fn hps_dns_recv4(ctx: SockAddrContext) -> i32 {
    restore(ctx, 4)
}
#[cgroup_sock_addr(recvmsg6)]
pub fn hps_dns_recv6(ctx: SockAddrContext) -> i32 {
    restore(ctx, 6)
}
#[cgroup_sock_addr(getpeername4)]
pub fn hps_dns_peer4(ctx: SockAddrContext) -> i32 {
    restore(ctx, 4)
}
#[cgroup_sock_addr(getpeername6)]
pub fn hps_dns_peer6(ctx: SockAddrContext) -> i32 {
    restore(ctx, 6)
}

#[inline(always)]
fn policy(ctx: &SockAddrContext) -> Option<DnsPolicy> {
    let policy = *HPS_DNS_POLICY.get(0)?;
    if policy.abi_version != DNS_ABI_VERSION || policy.flags & DNS_ENABLED == 0 {
        return None;
    }
    if (bpf_get_current_pid_tgid() >> 32) as u32 == policy.excluded_tgid {
        return None;
    }
    // SAFETY: kernel-provided sock_addr context; this helper identifies its socket's netns.
    if unsafe { bpf_get_netns_cookie(ctx.as_ptr()) } != policy.netns_cookie {
        return None;
    }
    Some(policy)
}
#[inline(always)]
fn redirect(ctx: SockAddrContext, family: u32) -> i32 {
    // SAFETY: each hook receives the matching family context. Inline constant family
    // branches prevent accessing IPv6 context words in an IPv4 program.
    let raw = unsafe { &mut *ctx.sock_addr };
    if u16::from_be(raw.user_port as u16) != 53 || (raw.protocol != 6 && raw.protocol != 17) {
        return 1;
    }
    let Some(policy) = policy(&ctx) else { return 1 };
    let index = if family == 4 { 0 } else { 1 };
    let Some(stats) = HPS_DNS_STATS.get_ptr_mut(index) else {
        return 0;
    };
    // SAFETY: fixed per-CPU value, non-sleepable hook, pointer never escapes.
    let stats = unsafe { &mut *stats };
    let peer = DnsPeer {
        family,
        address: if family == 4 {
            [raw.user_ip4, 0, 0, 0]
        } else {
            raw.user_ip6
        },
    };
    let slot = match slot(peer) {
        Ok(slot) => slot,
        Err((code, phase)) => {
            stats.errors = stats.errors.wrapping_add(1);
            stats.error_code = code;
            stats.error_phase = phase;
            return 0;
        }
    };
    if family == 4 {
        raw.user_ip4 = policy.alias4_prefix | slot.to_be();
    } else {
        // Volatile individual stores preserve fixed context offsets. A compiler
        // memcpy of the array can emit a modified ctx pointer rejected by Linux.
        unsafe {
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!(raw.user_ip6[0]),
                policy.alias6_prefix[0],
            );
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!(raw.user_ip6[1]),
                policy.alias6_prefix[1],
            );
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!(raw.user_ip6[2]),
                policy.alias6_prefix[2],
            );
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!(raw.user_ip6[3]),
                policy.alias6_prefix[3] | slot.to_be(),
            );
        }
    }
    raw.user_port = (policy.port as u16).to_be() as u32;
    stats.redirected = stats.redirected.wrapping_add(1);
    1
}
#[inline(always)]
fn slot(peer: DnsPeer) -> Result<u32, (i64, u64)> {
    // SAFETY: map lookup value is copied before map mutations; no pointers escape.
    if let Some(slot) = unsafe { HPS_DNS_PEERS.get(&peer) } {
        return Ok(*slot);
    }
    let allocation = HPS_DNS_ALLOC.get_ptr_mut(0).ok_or((-2, 1))?;
    // SAFETY: the Array value is u64-aligned and shared updates use BPF atomic FETCH_ADD.
    // Slots are never reused during a loader lifetime, including lost insertion races.
    // SeqCst is intentional: bpf-linker 0.12's LLVM 22 miscompiles a live relaxed
    // atomicrmw result into non-FETCH XADD (llvm-project #210280/#210371).
    // Real multi-peer kernel acceptance verifies distinct returned slot values.
    let next = unsafe {
        atomic_xadd::<u64, u64, { AtomicOrdering::SeqCst }>(
            core::ptr::addr_of_mut!((*allocation).next_slot),
            1,
        )
    };
    if next == 0 || next > 65535 {
        return Err((-28, 1));
    }
    let next = next as u32;
    HPS_DNS_REVERSE
        .insert(&next, &peer, 1)
        .map_err(|e| (i64::from(e), 2))?; // BPF_NOEXIST
    if let Err(error) = HPS_DNS_PEERS.insert(&peer, &next, 1) {
        let _ = HPS_DNS_REVERSE.remove(&next);
        return unsafe { HPS_DNS_PEERS.get(&peer) }
            .copied()
            .ok_or((i64::from(error), 3));
    }
    Ok(next)
}
#[inline(always)]
fn restore(ctx: SockAddrContext, family: u32) -> i32 {
    let Some(policy) = policy(&ctx) else { return 1 };
    // SAFETY: same per-attachment family restriction as redirect().
    let raw = unsafe { &mut *ctx.sock_addr };
    if u16::from_be(raw.user_port as u16) != policy.port as u16 {
        return 1;
    }
    let slot = if family == 4 {
        if raw.user_ip4 & 0x0000ffff != policy.alias4_prefix {
            return 1;
        }
        u32::from_be(raw.user_ip4) & 0xffff
    } else {
        if raw.user_ip6[0] != policy.alias6_prefix[0]
            || raw.user_ip6[1] != policy.alias6_prefix[1]
            || raw.user_ip6[2] != policy.alias6_prefix[2]
            || raw.user_ip6[3] & 0x0000ffff != policy.alias6_prefix[3]
        {
            return 1;
        }
        u32::from_be(raw.user_ip6[3]) & 0xffff
    };
    let Some(stats) = HPS_DNS_STATS.get_ptr_mut(if family == 4 { 0 } else { 1 }) else {
        return 1;
    };
    // SAFETY: fixed per-CPU map value, no sleeping or escaping pointer.
    let stats = unsafe { &mut *stats };
    let Some(peer) = unsafe { HPS_DNS_REVERSE.get(&slot) }.copied() else {
        stats.errors = stats.errors.wrapping_add(1);
        return 1;
    };
    if peer.family != family {
        stats.errors = stats.errors.wrapping_add(1);
        return 1;
    }
    if family == 4 {
        raw.user_ip4 = peer.address[0];
    } else {
        unsafe {
            core::ptr::write_volatile(core::ptr::addr_of_mut!(raw.user_ip6[0]), peer.address[0]);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(raw.user_ip6[1]), peer.address[1]);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(raw.user_ip6[2]), peer.address[2]);
            core::ptr::write_volatile(core::ptr::addr_of_mut!(raw.user_ip6[3]), peer.address[3]);
        }
    }
    raw.user_port = 53u16.to_be() as u32;
    stats.restored = stats.restored.wrapping_add(1);
    1
}
