//! Owns one DNS hook set and a private local alias route in this network namespace.
mod route;
use aya::{
    Ebpf, EbpfLoader,
    maps::{Array, HashMap, PerCpuArray},
    programs::{CgroupAttachMode, CgroupSockAddr},
};
use hps_domain::{DnsCaptureStats, DnsCaptureStatus};
use hps_ebpf_abi::{
    DNS_ABI_VERSION, DNS_ENABLED, DNS_PEER_CAPACITY, DnsAllocation, DnsPeer, DnsPolicy, DnsStats,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::{Ipv4Addr, Ipv6Addr, UdpSocket},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::Mutex,
};

const PROGRAMS: [&str; 8] = [
    "hps_dns_connect4",
    "hps_dns_connect6",
    "hps_dns_send4",
    "hps_dns_send6",
    "hps_dns_recv4",
    "hps_dns_recv6",
    "hps_dns_peer4",
    "hps_dns_peer6",
];
pub const IPV4_ALIAS_PREFIX: Ipv4Addr = Ipv4Addr::new(127, 77, 0, 0);
pub const IPV6_ALIAS_PREFIX: Ipv6Addr = Ipv6Addr::new(0xfd77, 0x6870, 0x7300, 0, 0, 0, 0, 0);
fn error(label: &str, error: impl std::fmt::Display) -> String {
    format!("DNS capture {label}: {error}")
}

pub struct PreparedCapture {
    object: Vec<u8>,
    pub fingerprint: String,
    pub cgroup: PathBuf,
    pub port: u16,
}
impl PreparedCapture {
    pub fn read(object: &Path, cgroup: &Path, port: u16) -> Result<Self, String> {
        if cfg!(target_endian = "big") {
            return Err("DNS capture object requires a little-endian Linux target".into());
        }
        if port == 0 || port == 53 {
            return Err(
                "DNS capture port must be nonzero and different from original port 53".into(),
            );
        }
        let cgroup = super::ebpf::cgroup_v2(cgroup)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(object)
            .map_err(|e| error("object", e))?;
        let info = file.metadata().map_err(|e| error("object metadata", e))?;
        if !info.is_file() || info.len() > 16 * 1024 * 1024 {
            return Err("DNS capture object must be a regular ELF file up to 16 MiB".into());
        }
        let mut bytes = vec![];
        file.take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| error("object read", e))?;
        if bytes.len() > 16 * 1024 * 1024 || !bytes.starts_with(b"\x7fELF") {
            return Err("DNS capture object is not a bounded ELF file".into());
        }
        let fingerprint = format!("{:x}", Sha256::digest(&bytes));
        Ok(Self {
            object: bytes,
            fingerprint,
            cgroup,
            port,
        })
    }
    pub fn load(self) -> Result<DnsCapture, String> {
        let netns_cookie = netns_cookie()?;
        let ownership = ownership(netns_cookie)?;
        let cgroup = File::open(&self.cgroup).map_err(|e| error("open cgroup", e))?;
        let mut ebpf = EbpfLoader::new()
            .map_max_entries("HPS_DNS_POLICY", 1)
            .map_max_entries("HPS_DNS_ALLOC", 1)
            .map_max_entries("HPS_DNS_PEERS", DNS_PEER_CAPACITY)
            .map_max_entries("HPS_DNS_REVERSE", DNS_PEER_CAPACITY)
            .map_max_entries("HPS_DNS_STATS", 2)
            .load(&self.object)
            .map_err(|e| error("parse ELF/maps", e))?;
        let pidns =
            std::fs::metadata("/proc/self/ns/pid").map_err(|e| error("PID namespace", e))?;
        let policy = DnsPolicy {
            abi_version: DNS_ABI_VERSION,
            flags: 0,
            port: u32::from(self.port),
            excluded_tgid: std::process::id(),
            netns_cookie,
            alias4_prefix: u32::from_ne_bytes(IPV4_ALIAS_PREFIX.octets()),
            reserved: 0,
            alias6_prefix: words(IPV6_ALIAS_PREFIX.octets()),
            pidns_device: pidns.dev(),
            pidns_inode: pidns.ino(),
        };
        {
            let mut array: Array<_, DnsPolicy> = Array::try_from(
                ebpf.map_mut("HPS_DNS_POLICY")
                    .ok_or("DNS policy map missing")?,
            )
            .map_err(|e| error("policy ABI", e))?;
            array
                .set(0, policy, 0)
                .map_err(|e| error("initialize policy", e))?;
            let mut allocation: Array<_, DnsAllocation> = Array::try_from(
                ebpf.map_mut("HPS_DNS_ALLOC")
                    .ok_or("DNS allocation map missing")?,
            )
            .map_err(|e| error("allocation ABI", e))?;
            allocation
                .set(0, DnsAllocation { next_slot: 1 }, 0)
                .map_err(|e| error("initialize allocator", e))?;
            let _: HashMap<_, DnsPeer, u32> =
                HashMap::try_from(ebpf.map("HPS_DNS_PEERS").ok_or("DNS peers missing")?)
                    .map_err(|e| error("peer ABI", e))?;
            let _: HashMap<_, u32, DnsPeer> = HashMap::try_from(
                ebpf.map("HPS_DNS_REVERSE")
                    .ok_or("DNS reverse peers missing")?,
            )
            .map_err(|e| error("reverse ABI", e))?;
            let _: PerCpuArray<_, DnsStats> =
                PerCpuArray::try_from(ebpf.map("HPS_DNS_STATS").ok_or("DNS stats missing")?)
                    .map_err(|e| error("stats ABI", e))?;
        }
        for name in PROGRAMS {
            let program: &mut CgroupSockAddr = ebpf
                .program_mut(name)
                .ok_or("DNS program missing")?
                .try_into()
                .map_err(|e| error("program type", e))?;
            program.load().map_err(|e| error("kernel verifier", e))?;
        }
        let route = route::LocalRoute::prepare(IPV6_ALIAS_PREFIX)
            .map_err(|e| error("owned IPv6 local route", e))?;
        Ok(DnsCapture {
            ebpf: Mutex::new(Some(ebpf)),
            cgroup,
            scope: self.cgroup,
            policy,
            fingerprint: self.fingerprint,
            route: Mutex::new(Some(route)),
            ownership: Mutex::new(Some(ownership)),
            attached: std::sync::atomic::AtomicBool::new(false),
        })
    }
}
pub struct DnsCapture {
    ebpf: Mutex<Option<Ebpf>>,
    cgroup: File,
    scope: PathBuf,
    policy: DnsPolicy,
    fingerprint: String,
    route: Mutex<Option<route::LocalRoute>>,
    ownership: Mutex<Option<File>>,
    attached: std::sync::atomic::AtomicBool,
}
impl DnsCapture {
    /// Called only after the two capture listeners have successfully started.
    pub fn activate(&self) -> Result<(), String> {
        let mut guard = self.ebpf.lock().unwrap();
        let ebpf = guard.as_mut().ok_or("DNS capture is shut down")?;
        if self.attached.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        for name in PROGRAMS {
            let program: &mut CgroupSockAddr = ebpf
                .program_mut(name)
                .ok_or("DNS program missing")?
                .try_into()
                .map_err(|e| error("program type", e))?;
            if let Err(e) = program.attach(&self.cgroup, CgroupAttachMode::Single) {
                // Taking Ebpf detaches any earlier links from this partial activation.
                guard.take();
                return Err(error("attach cgroup hooks", e));
            }
        }
        let mut policy = self.policy;
        policy.flags = DNS_ENABLED;
        let mut array: Array<_, DnsPolicy> =
            Array::try_from(ebpf.map_mut("HPS_DNS_POLICY").ok_or("DNS policy missing")?)
                .map_err(|e| error("policy ABI", e))?;
        array
            .set(0, policy, 0)
            .map_err(|e| error("enable hooks", e))?;
        self.attached
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub fn status(&self) -> Result<DnsCaptureStatus, String> {
        let guard = self.ebpf.lock().unwrap();
        let ebpf = guard.as_ref().ok_or("DNS capture is shut down")?;
        let mut program_ids = vec![];
        let mut maps = BTreeSet::new();
        for name in PROGRAMS {
            let info = ebpf
                .program(name)
                .ok_or("DNS program missing")?
                .info()
                .map_err(|e| error("program info", e))?;
            program_ids.push(info.id());
            maps.extend(
                info.map_ids()
                    .map_err(|e| error("map info", e))?
                    .ok_or("DNS kernel did not report maps")?,
            );
        }
        let peers: HashMap<_, DnsPeer, u32> =
            HashMap::try_from(ebpf.map("HPS_DNS_PEERS").ok_or("DNS peers missing")?)
                .map_err(|e| error("peer ABI", e))?;
        let peers = peers
            .iter()
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| error("read peers", e))?
            .len();
        let allocation: Array<_, DnsAllocation> =
            Array::try_from(ebpf.map("HPS_DNS_ALLOC").ok_or("DNS allocator missing")?)
                .map_err(|e| error("allocator ABI", e))?;
        let allocated_slots = allocation
            .get(&0, 0)
            .map_err(|e| error("read allocator", e))?
            .next_slot
            .saturating_sub(1);
        let stats: PerCpuArray<_, DnsStats> =
            PerCpuArray::try_from(ebpf.map("HPS_DNS_STATS").ok_or("DNS stats missing")?)
                .map_err(|e| error("stats ABI", e))?;
        let mut totals = vec![];
        for index in 0..2 {
            let values = stats.get(&index, 0).map_err(|e| error("read stats", e))?;
            let mut sum = DnsStats::default();
            for value in values.iter() {
                sum.redirected += value.redirected;
                sum.restored += value.restored;
                sum.errors += value.errors;
                sum.bypassed += value.bypassed;
                if value.error_code != 0 {
                    sum.error_code = value.error_code;
                    sum.error_phase = value.error_phase;
                }
            }
            totals.push(DnsCaptureStats {
                family: if index == 0 { "ipv4" } else { "ipv6" }.into(),
                redirected: sum.redirected,
                restored: sum.restored,
                errors: sum.errors,
                bypassed: sum.bypassed,
                error_code: sum.error_code,
                error_phase: sum.error_phase,
            });
        }
        Ok(DnsCaptureStatus {
            scope: "cgroup_local_application_dns".into(),
            attached: self.attached.load(std::sync::atomic::Ordering::Acquire),
            object_sha256: self.fingerprint.clone(),
            cgroup: self.scope.display().to_string(),
            port: self.policy.port as u16,
            ipv4_alias_prefix: format!("{IPV4_ALIAS_PREFIX}/16"),
            ipv6_alias_prefix: format!("{IPV6_ALIAS_PREFIX}/112"),
            netns_cookie: self.policy.netns_cookie,
            program_ids,
            map_ids: maps.into_iter().collect(),
            peers,
            allocated_slots,
            stats: totals,
        })
    }
    pub fn shutdown(&self) -> Result<(), String> {
        self.ebpf.lock().unwrap().take();
        self.attached
            .store(false, std::sync::atomic::Ordering::Release);
        if let Some(route) = self.route.lock().unwrap().take() {
            route.remove().map_err(|e| error("remove owned route", e))?;
            if route
                .exists()
                .map_err(|e| error("verify owned route removal", e))?
            {
                return Err("DNS capture owned route was not removed".into());
            }
        }
        self.ownership.lock().unwrap().take();
        Ok(())
    }
}
impl Drop for DnsCapture {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
fn words(bytes: [u8; 16]) -> [u32; 4] {
    std::array::from_fn(|i| u32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()))
}
fn netns_cookie() -> Result<u64, String> {
    let socket = UdpSocket::bind("127.0.0.1:0").map_err(|e| error("namespace socket", e))?;
    let mut value = 0u64;
    let mut len = std::mem::size_of_val(&value) as libc::socklen_t;
    // SAFETY: live UDP descriptor and correctly sized output storage.
    if unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NETNS_COOKIE,
            (&mut value as *mut u64).cast(),
            &mut len,
        )
    } != 0
    {
        return Err(error("namespace cookie", std::io::Error::last_os_error()));
    }
    if value == 0 || len as usize != std::mem::size_of::<u64>() {
        return Err("DNS capture invalid namespace cookie".into());
    }
    Ok(value)
}
fn ownership(cookie: u64) -> Result<File, String> {
    let path = Path::new("/run/hps");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| error("ownership directory", e))?;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| error("ownership directory metadata", e))?;
    // This is OS object ownership, independent of the project's intentionally open management API.
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err("DNS capture requires its own private /run/hps directory".into());
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path.join(format!("dns-netns-{cookie}.lock")))
        .map_err(|e| error("ownership file", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| error("ownership file metadata", e))?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(
            "DNS capture ownership file must be private and owned by the daemon OS user".into(),
        );
    }
    fs2::FileExt::try_lock_exclusive(&file)
        .map_err(|e| error("another DNS capture owns this network namespace", e))?;
    file.set_len(0).map_err(|e| error("ownership record", e))?;
    file.write_all(format!("{}\n", std::process::id()).as_bytes())
        .map_err(|e| error("ownership record", e))?;
    Ok(file)
}
