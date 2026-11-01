//! Versioned plain-data boundary shared by our Rust hooks and loader.
#![no_std]
pub const ABI_VERSION: u32 = 1;
pub const FLAG_DENY_TEST_PORT: u32 = 1;
pub const OBSERVATION_CAPACITY: u32 = 128;
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Policy {
    pub abi_version: u32,
    pub flags: u32,
    pub deny_port: u32,
    pub reserved: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Stats {
    pub connects: u64,
    pub allowed: u64,
    pub denied: u64,
    pub errors: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Observation {
    pub abi_version: u32,
    pub family: u32,
    pub process_id: u32,
    pub thread_id: u32,
    pub cgroup_id: u64,
    pub socket_cookie: u64,
    /// Linux network-order address words, copied without reinterpretation.
    pub destination: [u32; 4],
    pub port: u32,
    pub protocol: u32,
    pub command: [u8; 16],
}
const _: () = assert!(core::mem::size_of::<Policy>() == 16);
const _: () = assert!(core::mem::size_of::<Stats>() == 32);
const _: () = assert!(core::mem::size_of::<Observation>() == 72);
#[cfg(feature = "userspace")]
// SAFETY: repr(C), integer/byte fields, all bytes initialized, no references or invalid bit patterns.
unsafe impl aya::Pod for Policy {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), four u64 fields and no padding.
unsafe impl aya::Pod for Stats {}
#[cfg(feature = "userspace")]
// SAFETY: repr(C), integer/byte fields, explicit layout assertions, no padding or references.
unsafe impl aya::Pod for Observation {}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_layout_has_no_implicit_padding() {
        assert_eq!(core::mem::align_of::<Observation>(), 8);
        assert_eq!(core::mem::offset_of!(Observation, destination), 32);
        assert_eq!(core::mem::offset_of!(Observation, command), 56);
        assert_eq!(core::mem::size_of::<Observation>(), 72);
    }
}
