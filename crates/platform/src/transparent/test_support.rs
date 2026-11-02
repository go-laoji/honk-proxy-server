use crate::rtnetlink::{self, attribute};
use std::io::Write;
use std::{net::IpAddr, path::PathBuf};
pub(super) struct Address {
    payload: Vec<u8>,
}
impl Address {
    pub(super) fn add(address: &str) -> Self {
        let ip = address.parse::<IpAddr>().unwrap();
        let (family, prefix, flags, bytes) = match ip {
            IpAddr::V4(a) => (libc::AF_INET as u8, 32, 0, a.octets().to_vec()),
            IpAddr::V6(a) => (libc::AF_INET6 as u8, 128, 2, a.octets().to_vec()), // NODAD
        };
        let mut payload = vec![family, prefix, flags, 0];
        payload.extend_from_slice(&unsafe { libc::if_nametoindex(c"lo".as_ptr()) }.to_ne_bytes());
        attribute(&mut payload, 1, &bytes);
        attribute(&mut payload, 2, &bytes);
        rtnetlink::exchange(
            libc::RTM_NEWADDR,
            (libc::NLM_F_REQUEST | libc::NLM_F_ACK | libc::NLM_F_CREATE | libc::NLM_F_EXCL) as u16,
            &payload,
            false,
        )
        .unwrap();
        Self { payload }
    }
}
impl Drop for Address {
    fn drop(&mut self) {
        let _ = rtnetlink::exchange(
            libc::RTM_DELADDR,
            (libc::NLM_F_REQUEST | libc::NLM_F_ACK) as u16,
            &self.payload,
            false,
        );
    }
}
pub(super) struct Group(pub(super) PathBuf);
impl Drop for Group {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
pub(super) struct Jobs(pub(super) Vec<tokio::task::JoinHandle<()>>);
impl Drop for Jobs {
    fn drop(&mut self) {
        for job in &self.0 {
            job.abort();
        }
    }
}

pub(super) struct Membership(PathBuf);
impl Membership {
    pub(super) fn enter(group: &std::path::Path) -> Self {
        let root = PathBuf::from(std::env::var("HPS_TEST_CGROUP_ROOT").unwrap());
        let previous = std::fs::read_to_string("/proc/self/cgroup").unwrap();
        let previous = previous
            .lines()
            .find_map(|l| l.strip_prefix("0::"))
            .unwrap();
        assert!(
            !std::path::Path::new(previous)
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
        );
        let previous = root.join(previous.trim_start_matches('/'));
        Self::write(group);
        Self(previous)
    }
    fn write(group: &std::path::Path) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(group.join("cgroup.procs"))
            .unwrap()
            .write_all(format!("{}\n", std::process::id()).as_bytes())
            .unwrap();
    }
}
impl Drop for Membership {
    fn drop(&mut self) {
        Self::write(&self.0);
    }
}
