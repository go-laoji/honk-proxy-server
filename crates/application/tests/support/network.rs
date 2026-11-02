use crate::rtnetlink;
use std::{ffi::CString, net::IpAddr, os::fd::AsRawFd};
pub(super) fn index(name: &str) -> u32 {
    let value = unsafe { libc::if_nametoindex(CString::new(name).unwrap().as_ptr()) };
    assert_ne!(value, 0, "missing test link {name}");
    value
}
pub(super) fn link_header(index: u32) -> Vec<u8> {
    let mut bytes = vec![0; 16];
    bytes[4..8].copy_from_slice(&index.to_ne_bytes());
    bytes
}
pub(super) fn request(kind: u16, payload: &[u8], create: bool) {
    let flags = libc::NLM_F_REQUEST
        | libc::NLM_F_ACK
        | if create {
            libc::NLM_F_CREATE | libc::NLM_F_EXCL
        } else {
            0
        };
    rtnetlink::exchange(kind, flags as u16, payload, false).unwrap();
}
pub(super) fn up(name: &str) {
    set_up(index(name), true);
}
pub(super) fn set_up(index: u32, enabled: bool) {
    let mut payload = link_header(index);
    payload[8..12].copy_from_slice(&(if enabled { libc::IFF_UP as u32 } else { 0 }).to_ne_bytes());
    payload[12..16].copy_from_slice(&(libc::IFF_UP as u32).to_ne_bytes());
    request(libc::RTM_NEWLINK, &payload, false);
}
pub(super) fn rename(index: u32, name: &str) {
    let mut payload = link_header(index);
    let name = CString::new(name).unwrap();
    rtnetlink::attribute(&mut payload, 3, name.as_bytes_with_nul());
    request(libc::RTM_NEWLINK, &payload, false);
}
pub(super) fn address(name: &str, ip: &str) {
    let ip = ip.parse::<IpAddr>().unwrap();
    let (family, prefix, bytes) = match ip {
        IpAddr::V4(ip) => (libc::AF_INET, 24, ip.octets().to_vec()),
        IpAddr::V6(ip) => (libc::AF_INET6, 64, ip.octets().to_vec()),
    };
    let mut payload = vec![family as u8, prefix, if ip.is_ipv6() { 2 } else { 0 }, 0];
    payload.extend_from_slice(&index(name).to_ne_bytes());
    rtnetlink::attribute(&mut payload, 1, &bytes);
    rtnetlink::attribute(&mut payload, 2, &bytes);
    request(libc::RTM_NEWADDR, &payload, true);
}
pub(super) fn default_route(gateway: &str) {
    let ip = gateway.parse::<IpAddr>().unwrap();
    let (family, bytes) = match ip {
        IpAddr::V4(ip) => (libc::AF_INET, ip.octets().to_vec()),
        IpAddr::V6(ip) => (libc::AF_INET6, ip.octets().to_vec()),
    };
    let mut payload = vec![family as u8, 0, 0, 0, 254, 4, 0, 1, 0, 0, 0, 0];
    rtnetlink::attribute(&mut payload, 5, &bytes);
    rtnetlink::attribute(&mut payload, 4, &index("hpsclient0").to_ne_bytes());
    request(libc::RTM_NEWROUTE, &payload, true);
}
pub(super) struct Veth(pub(super) u32);
impl Veth {
    pub(super) fn create(namespace: &std::fs::File) -> Self {
        let mut peer = link_header(0);
        rtnetlink::attribute(&mut peer, 3, b"hpsclient0\0");
        let mut data = vec![];
        rtnetlink::attribute(&mut data, 1 | 0x8000, &peer);
        let mut info = vec![];
        rtnetlink::attribute(&mut info, 1, b"veth\0");
        rtnetlink::attribute(&mut info, 2 | 0x8000, &data);
        let mut payload = link_header(0);
        rtnetlink::attribute(&mut payload, 3, b"hpslan0\0");
        rtnetlink::attribute(&mut payload, 18 | 0x8000, &info);
        request(libc::RTM_NEWLINK, &payload, true);
        let result = Self(index("hpslan0"));
        let mut payload = link_header(index("hpsclient0"));
        rtnetlink::attribute(&mut payload, 28, &namespace.as_raw_fd().to_ne_bytes());
        request(libc::RTM_NEWLINK, &payload, false);
        up("hpslan0");
        address("hpslan0", "10.231.83.1");
        address("hpslan0", "fd42:6870:7300:1::1");
        result
    }
}
impl Drop for Veth {
    fn drop(&mut self) {
        let _ = rtnetlink::exchange(
            libc::RTM_DELLINK,
            (libc::NLM_F_REQUEST | libc::NLM_F_ACK) as u16,
            &link_header(self.0),
            false,
        );
    }
}
