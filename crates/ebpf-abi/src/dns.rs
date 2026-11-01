//! DNS aliases identify the original resolver, not a target-site FakeIP.
pub const DNS_ABI_VERSION: u32 = 1;
pub const DNS_PEER_CAPACITY: u32 = 4096;
pub const DNS_ENABLED: u32 = 1;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DnsPolicy {
    pub abi_version: u32,
    pub flags: u32,
    pub port: u32,
    pub excluded_tgid: u32,
    pub netns_cookie: u64,
    /// Network-order address words. Low 16 address bits are zero in each prefix.
    pub alias4_prefix: u32,
    pub reserved: u32,
    pub alias6_prefix: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DnsPeer {
    pub family: u32,
    pub address: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DnsAllocation {
    pub next_slot: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DnsStats {
    pub redirected: u64,
    pub restored: u64,
    pub errors: u64,
    pub bypassed: u64,
    pub error_code: i64,
    pub error_phase: u64,
}
const _: () = assert!(core::mem::size_of::<DnsPolicy>() == 48);
const _: () = assert!(core::mem::size_of::<DnsPeer>() == 20);
const _: () = assert!(core::mem::size_of::<DnsStats>() == 48);
#[cfg(feature = "userspace")]
// SAFETY: repr(C), integer fields only; explicit reserved fills all alignment padding.
unsafe impl aya::Pod for DnsPolicy {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), five u32 fields and no padding.
unsafe impl aya::Pod for DnsPeer {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), one u64 field.
unsafe impl aya::Pod for DnsAllocation {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), four u64 fields.
unsafe impl aya::Pod for DnsStats {}
