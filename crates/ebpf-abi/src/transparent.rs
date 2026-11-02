//! Shared, fixed-width transparent capture boundary. Addresses are network-order
//! words; ports are host-order. Neither kernel pointers nor payload bytes cross it.
// v4 adds a frozen ABI marker and explicit suspended capture semantics.
pub const TP_ABI_VERSION: u32 = 4;
pub const TP_ABI_MAP: &str = "HPS_TP_ABI4";
pub const TP_ENABLED: u32 = 1;
pub const TP_TCP: u32 = 2;
pub const TP_UDP: u32 = 4;
pub const TP_IPV4: u32 = 8;
pub const TP_IPV6: u32 = 16;
pub const TP_REQUIRE_PROCESS: u32 = 32;
pub const TP_SUSPENDED: u32 = 64;
pub const TP_PROCESS_VERIFIED: u32 = 1;
pub const TP_LOCAL: u32 = 1;
pub const TP_GATEWAY: u32 = 2;
pub const TP_SOCKET_TCP4: u32 = 0;
pub const TP_SOCKET_TCP6: u32 = 1;
pub const TP_SOCKET_UDP4: u32 = 2;
pub const TP_SOCKET_UDP6: u32 = 3;
pub const TP_SOCKET_COUNT: u32 = 4;
pub const TP_FLOW_CAPACITY: u32 = 8192;
pub const TP_MAX_INTERFACES: u32 = 16;
pub const TP_MAX_BYPASS: u32 = 256;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CapturePolicy {
    pub abi_version: u32,
    pub flags: u32,
    pub revision: u64,
    pub netns_cookie: u64,
    pub pidns_device: u64,
    pub pidns_inode: u64,
    pub excluded_tgid: u32,
    pub capture_mark: u32,
    pub bypass_mark: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub family: u32,
    pub protocol: u32,
    pub source: [u32; 4],
    pub destination: [u32; 4],
    pub source_port: u32,
    pub destination_port: u32,
    pub netns_cookie: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SocketProcess {
    pub socket_cookie: u64,
    pub cgroup_id: u64,
    pub observed_at_ns: u64,
    pub pid: u32,
    pub thread_id: u32,
    pub command: [u8; 16],
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CapturedFlow {
    pub revision: u64,
    pub first_seen_ns: u64,
    pub last_seen_ns: u64,
    pub process: SocketProcess,
    pub origin: u32,
    pub flags: u32,
    pub initial_sequence: u32,
    pub reserved: u32,
}
impl CapturedFlow {
    /// Activity may advance concurrently; every other field belongs to the
    /// immutable tuple claim and must match before its owner can retire it.
    pub fn same_identity(&self, other: &Self) -> bool {
        Self {
            last_seen_ns: 0,
            ..*self
        } == Self {
            last_seen_ns: 0,
            ..*other
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CaptureStats {
    pub observed_sockets: u64,
    pub marked_sockets: u64,
    pub captured_packets: u64,
    pub bypassed_packets: u64,
    pub rejected_packets: u64,
    pub unsupported_packets: u64,
    pub missing_process: u64,
    pub errors: u64,
    pub last_error: i64,
    pub error_phase: u64,
}
const _: () = assert!(core::mem::size_of::<CapturePolicy>() == 56);
const _: () = assert!(core::mem::size_of::<FlowKey>() == 56);
const _: () = assert!(core::mem::size_of::<SocketProcess>() == 48);
const _: () = assert!(core::mem::size_of::<CapturedFlow>() == 88);
const _: () = assert!(core::mem::size_of::<CaptureStats>() == 80);

#[cfg(feature = "userspace")]
// SAFETY: repr(C), only integer fields, explicit reserved storage, layout asserted above.
unsafe impl aya::Pod for CapturePolicy {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), initialized integer/array fields, no implicit padding.
unsafe impl aya::Pod for FlowKey {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), initialized integer/array fields, no implicit padding.
unsafe impl aya::Pod for SocketProcess {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), nested SocketProcess has no padding; reserved fills alignment.
unsafe impl aya::Pod for CapturedFlow {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), ten initialized 64-bit integers and no padding.
unsafe impl aya::Pod for CaptureStats {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_boundary_offsets_and_sizes_match_kernel_map_contract() {
        assert_eq!(core::mem::offset_of!(CapturePolicy, revision), 8);
        assert_eq!(core::mem::offset_of!(CapturePolicy, excluded_tgid), 40);
        assert_eq!(core::mem::offset_of!(FlowKey, netns_cookie), 48);
        assert_eq!(core::mem::offset_of!(SocketProcess, command), 32);
        assert_eq!(core::mem::offset_of!(CapturedFlow, process), 24);
        assert_eq!(core::mem::offset_of!(CapturedFlow, origin), 72);
        assert_eq!(core::mem::offset_of!(CapturedFlow, reserved), 84);
        assert_eq!(core::mem::align_of::<CapturedFlow>(), 8);
    }
}
