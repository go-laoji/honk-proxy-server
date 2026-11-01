#![no_std]
#![no_main]
#![feature(core_intrinsics)]
#![allow(internal_features)]
mod dns;
mod transparent;
use aya_ebpf::{
    EbpfContext,
    helpers::{
        bpf_get_current_cgroup_id, bpf_get_current_comm, bpf_get_current_pid_tgid,
        bpf_get_socket_cookie,
    },
    macros::{cgroup_sock_addr, map},
    maps::{Array, LruHashMap, PerCpuArray},
    programs::SockAddrContext,
};
use hps_ebpf_abi::{
    ABI_VERSION, FLAG_DENY_TEST_PORT, OBSERVATION_CAPACITY, Observation, Policy, Stats,
};
#[map]
static HPS_POLICY: Array<Policy> = Array::with_max_entries(1, 0);
#[map]
static HPS_STATS: PerCpuArray<Stats> = PerCpuArray::with_max_entries(2, 0);
#[map]
static HPS_OBSERVE: LruHashMap<u64, Observation> =
    LruHashMap::with_max_entries(OBSERVATION_CAPACITY, 0);
#[cgroup_sock_addr(connect4)]
pub fn hps_connect4(ctx: SockAddrContext) -> i32 {
    observe(ctx, 0, 4)
}
#[cgroup_sock_addr(connect6)]
pub fn hps_connect6(ctx: SockAddrContext) -> i32 {
    observe(ctx, 1, 6)
}
#[inline(always)]
fn observe(ctx: SockAddrContext, index: u32, family: u32) -> i32 {
    let Some(stats) = HPS_STATS.get_ptr_mut(index) else {
        return 1;
    };
    // SAFETY: fixed per-CPU map index. Hooks are non-sleepable; the pointer never escapes.
    let stats = unsafe { &mut *stats };
    stats.connects = stats.connects.wrapping_add(1);
    // Copy the policy value instead of retaining its map reference across helpers.
    let policy = match HPS_POLICY.get(0) {
        Some(value) => *value,
        None => {
            stats.errors = stats.errors.wrapping_add(1);
            return 1;
        }
    };
    if policy.abi_version != ABI_VERSION {
        stats.errors = stats.errors.wrapping_add(1);
        return 1;
    }
    // SAFETY: SockAddrContext is provided by the kernel for these two attachment types.
    let raw = unsafe { &*ctx.sock_addr };
    let process = bpf_get_current_pid_tgid();
    let cookie = unsafe { bpf_get_socket_cookie(ctx.as_ptr()) };
    let port = u16::from_be(raw.user_port as u16) as u32;
    let destination = if family == 4 {
        [raw.user_ip4, 0, 0, 0]
    } else {
        raw.user_ip6
    };
    let observation = Observation {
        abi_version: ABI_VERSION,
        family,
        process_id: (process >> 32) as u32,
        thread_id: process as u32,
        cgroup_id: unsafe { bpf_get_current_cgroup_id() },
        socket_cookie: cookie,
        destination,
        port,
        protocol: raw.protocol,
        command: bpf_get_current_comm().unwrap_or([0; 16]),
    };
    if cookie == 0 || HPS_OBSERVE.insert(&cookie, &observation, 0).is_err() {
        stats.errors = stats.errors.wrapping_add(1);
    }
    if policy.flags & FLAG_DENY_TEST_PORT != 0 && policy.deny_port == port {
        stats.denied = stats.denied.wrapping_add(1);
        0
    } else {
        stats.allowed = stats.allowed.wrapping_add(1);
        1
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
pub static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
