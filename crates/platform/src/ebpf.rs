//! Real Linux hook ownership. This is intentionally separate from read-only preflight.
use aya::{
    Ebpf, EbpfLoader,
    maps::{Array, HashMap, MapError, MapInfo, PerCpuArray},
    programs::{CgroupAttachMode, CgroupSockAddr, loaded_programs},
};
use hps_domain::{EbpfFlowObservation, EbpfHookStats, LinuxEbpfSelfTest};
use hps_ebpf_abi::{ABI_VERSION, FLAG_DENY_TEST_PORT, Observation, Policy, Stats};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    io::Write,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn context(label: &str, error: impl std::fmt::Display) -> String {
    format!("{label}: {error}")
}
pub(super) fn cgroup_v2(path: &Path) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStrExt;
    let path = path.canonicalize().map_err(|e| context("cgroup path", e))?;
    let name =
        std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|_| "invalid cgroup path")?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: nul-terminated path and writable output storage; result read only after success.
    if unsafe { libc::statfs(name.as_ptr(), info.as_mut_ptr()) } != 0 {
        return Err(context(
            "cgroup filesystem",
            std::io::Error::last_os_error(),
        ));
    }
    // The libc field is signed on glibc and unsigned on musl.
    #[allow(
        clippy::unnecessary_cast,
        reason = "statfs ABI differs between libc implementations"
    )]
    let is_cgroup = unsafe { info.assume_init() }.f_type as u64 == libc::CGROUP2_SUPER_MAGIC as u64;
    if !is_cgroup {
        return Err("self-test requires a real cgroup v2 filesystem".into());
    }
    Ok(path)
}
struct OwnedCgroup {
    path: PathBuf,
}
impl OwnedCgroup {
    fn create(root: &Path) -> Result<Self, String> {
        let root = cgroup_v2(root)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| context("system clock", e))?
            .as_nanos();
        let path = root.join(format!("hps-self-test-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).map_err(|e| context("create isolated cgroup", e))?;
        Ok(Self { path })
    }
    fn remove(&self) -> Result<(), String> {
        std::fs::remove_dir(&self.path).map_err(|e| context("remove owned cgroup", e))
    }
}
impl Drop for OwnedCgroup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}
/// Closing this value detaches only the links this loader owns. No pins are created.
pub struct LoadedHooks {
    ebpf: Ebpf,
    _cgroup: File,
}
impl LoadedHooks {
    pub fn load(object: &[u8], cgroup: &Path) -> Result<Self, String> {
        let cgroup = cgroup_v2(cgroup)?;
        if !cgroup
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("hps-self-test-"))
        {
            return Err("isolated loader refuses a root or unrelated cgroup".into());
        }
        let file = File::open(cgroup).map_err(|e| context("open owned cgroup", e))?;
        let mut ebpf = EbpfLoader::new()
            .map_max_entries("HPS_POLICY", 1)
            .map_max_entries("HPS_STATS", 2)
            .map_max_entries("HPS_OBSERVE", hps_ebpf_abi::OBSERVATION_CAPACITY)
            .load(object)
            .map_err(|e| context("parse eBPF object and create maps", e))?;
        {
            let _: PerCpuArray<_, Stats> =
                PerCpuArray::try_from(ebpf.map("HPS_STATS").ok_or("missing stats map")?)
                    .map_err(|e| context("stats ABI", e))?;
            let _: HashMap<_, u64, Observation> =
                HashMap::try_from(ebpf.map("HPS_OBSERVE").ok_or("missing observation map")?)
                    .map_err(|e| context("observation ABI", e))?;
        }
        {
            let mut policy: Array<_, Policy> =
                Array::try_from(ebpf.map_mut("HPS_POLICY").ok_or("missing policy map")?)
                    .map_err(|e| context("policy ABI", e))?;
            policy
                .set(
                    0,
                    Policy {
                        abi_version: ABI_VERSION,
                        ..Default::default()
                    },
                    0,
                )
                .map_err(|e| context("initialize policy ABI", e))?;
        }
        for name in ["hps_connect4", "hps_connect6"] {
            let program: &mut CgroupSockAddr = ebpf
                .program_mut(name)
                .ok_or("missing connect program")?
                .try_into()
                .map_err(|e| context("program type", e))?;
            program.load().map_err(|e| context("kernel verifier", e))?;
            program
                .attach(&file, CgroupAttachMode::Single)
                .map_err(|e| context("attach isolated connect hook", e))?;
        }
        Ok(Self {
            ebpf,
            _cgroup: file,
        })
    }
    pub fn test_policy(&mut self, deny_port: Option<u16>) -> Result<(), String> {
        let mut policy: Array<_, Policy> = Array::try_from(
            self.ebpf
                .map_mut("HPS_POLICY")
                .ok_or("missing policy map")?,
        )
        .map_err(|e| context("policy ABI", e))?;
        policy
            .set(
                0,
                Policy {
                    abi_version: ABI_VERSION,
                    flags: if deny_port.is_some() {
                        FLAG_DENY_TEST_PORT
                    } else {
                        0
                    },
                    deny_port: u32::from(deny_port.unwrap_or(0)),
                    reserved: 0,
                },
                0,
            )
            .map_err(|e| context("update policy map", e))
    }
    fn ids(&self) -> Result<(Vec<u32>, Vec<u32>), String> {
        let mut programs = vec![];
        let mut maps = BTreeSet::new();
        for name in ["hps_connect4", "hps_connect6"] {
            let info = self
                .ebpf
                .program(name)
                .ok_or("missing loaded program")?
                .info()
                .map_err(|e| context("program info", e))?;
            programs.push(info.id());
            maps.extend(
                info.map_ids()
                    .map_err(|e| context("program maps", e))?
                    .ok_or("kernel cannot report map IDs")?,
            );
        }
        if maps.len() < 3 {
            return Err("loaded hooks did not reference all ABI maps".into());
        }
        Ok((programs, maps.into_iter().collect()))
    }
    fn stats(&self) -> Result<Vec<EbpfHookStats>, String> {
        let array: PerCpuArray<_, Stats> =
            PerCpuArray::try_from(self.ebpf.map("HPS_STATS").ok_or("missing stats")?)
                .map_err(|e| context("stats ABI", e))?;
        (0..2)
            .map(|index| {
                let values = array
                    .get(&index, 0)
                    .map_err(|e| context("read per-CPU counters", e))?;
                let mut stats = Stats::default();
                for value in values.iter() {
                    stats.connects = stats.connects.saturating_add(value.connects);
                    stats.allowed = stats.allowed.saturating_add(value.allowed);
                    stats.denied = stats.denied.saturating_add(value.denied);
                    stats.errors = stats.errors.saturating_add(value.errors);
                }
                Ok(EbpfHookStats {
                    family: if index == 0 { "ipv4" } else { "ipv6" }.into(),
                    connects: stats.connects,
                    allowed: stats.allowed,
                    denied: stats.denied,
                    errors: stats.errors,
                })
            })
            .collect()
    }
    fn observations(&self) -> Result<Vec<EbpfFlowObservation>, String> {
        let map: HashMap<_, u64, Observation> =
            HashMap::try_from(self.ebpf.map("HPS_OBSERVE").ok_or("missing observations")?)
                .map_err(|e| context("observation ABI", e))?;
        map.iter()
            .map(|entry| {
                let (cookie, value) = entry.map_err(|e| context("read observation", e))?;
                if value.abi_version != ABI_VERSION || cookie != value.socket_cookie {
                    return Err("observation ABI/cookie mismatch".into());
                }
                let ip = match value.family {
                    4 => IpAddr::V4(Ipv4Addr::from(u32::from_be(value.destination[0]))),
                    6 => {
                        let mut bytes = [0; 16];
                        for (index, word) in value.destination.iter().enumerate() {
                            bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_ne_bytes());
                        }
                        IpAddr::V6(Ipv6Addr::from(bytes))
                    }
                    _ => return Err("unknown observed address family".into()),
                };
                let end = value
                    .command
                    .iter()
                    .position(|b| *b == 0)
                    .unwrap_or(value.command.len());
                Ok(EbpfFlowObservation {
                    socket_cookie: cookie,
                    process_id: value.process_id,
                    thread_id: value.thread_id,
                    cgroup_id: value.cgroup_id,
                    family: if value.family == 4 { "ipv4" } else { "ipv6" }.into(),
                    destination: ip.to_string(),
                    port: u16::try_from(value.port).map_err(|_| "invalid observed port")?,
                    protocol: value.protocol,
                    command: String::from_utf8_lossy(&value.command[..end]).into_owned(),
                })
            })
            .collect()
    }
}
fn child(exe: &Path, cgroup: &Path, target: SocketAddr, deny: bool) -> Result<(), String> {
    let mut command = Command::new(exe);
    command
        .arg("--output")
        .arg("json")
        .arg("linux")
        .arg("self-test-child")
        .arg("--cgroup")
        .arg(cgroup)
        .arg("--target")
        .arg(target.to_string())
        .stdin(Stdio::null());
    if deny {
        command.arg("--expect-denied");
    }
    let output = command
        .output()
        .map_err(|e| context("spawn isolated test helper", e))?;
    if !output.status.success() {
        return Err(context(
            "test helper",
            String::from_utf8_lossy(&output.stderr),
        ));
    }
    Ok(())
}
pub fn self_test_child(
    cgroup: &Path,
    target: SocketAddr,
    expect_denied: bool,
) -> Result<(), String> {
    let path = cgroup_v2(cgroup)?;
    if !path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("hps-self-test-"))
    {
        return Err("helper refuses unrelated cgroup".into());
    }
    let mut membership = OpenOptions::new()
        .write(true)
        .open(path.join("cgroup.procs"))
        .map_err(|e| context("open helper membership", e))?;
    // cgroup.procs is a command interface: PID and newline must be one write.
    let pid = format!("{}\n", std::process::id());
    membership
        .write_all(pid.as_bytes())
        .map_err(|e| context("move only test helper", e))?;
    match TcpStream::connect_timeout(&target, Duration::from_secs(3)) {
        Ok(_) if !expect_denied => Ok(()),
        Err(error) if expect_denied && error.kind() == std::io::ErrorKind::PermissionDenied => {
            Ok(())
        }
        Ok(_) => Err("kernel map deny policy was not enforced".into()),
        Err(error) => Err(context("test connect", error)),
    }
}
pub fn self_test(
    object: &Path,
    cgroup_root: &Path,
    helper: &Path,
) -> Result<LinuxEbpfSelfTest, String> {
    let size = std::fs::metadata(object).map_err(|e| context("object metadata", e))?;
    if !size.is_file() || size.len() > 4 * 1024 * 1024 {
        return Err("object must be a regular ELF file no larger than 4 MiB".into());
    }
    let object = std::fs::read(object).map_err(|e| context("read object", e))?;
    let ipv4 = TcpListener::bind("127.0.0.1:0").map_err(|e| context("bind IPv4 test", e))?;
    let ipv6 = TcpListener::bind("[::1]:0").map_err(|e| context("bind IPv6 test", e))?;
    let v4 = ipv4
        .local_addr()
        .map_err(|e| context("IPv4 test address", e))?;
    let v6 = ipv6
        .local_addr()
        .map_err(|e| context("IPv6 test address", e))?;
    let group = OwnedCgroup::create(cgroup_root)?;
    let mut hooks = LoadedHooks::load(&object, &group.path)?;
    child(helper, &group.path, v4, false)?;
    child(helper, &group.path, v6, false)?;
    hooks.test_policy(Some(v4.port()))?;
    child(helper, &group.path, v4, true)?;
    hooks.test_policy(None)?;
    child(helper, &group.path, v4, false)?;
    let stats = hooks.stats()?;
    if stats[0].allowed < 2
        || stats[0].denied < 1
        || stats[1].allowed < 1
        || stats.iter().any(|s| s.errors != 0)
    {
        return Err("kernel counters do not prove expected real connections".into());
    }
    let observations = hooks.observations()?;
    if !observations
        .iter()
        .any(|o| o.destination == "127.0.0.1" && o.port == v4.port())
        || !observations
            .iter()
            .any(|o| o.destination == "::1" && o.port == v6.port())
        || observations
            .iter()
            .any(|o| o.process_id == 0 || o.cgroup_id == 0 || o.command.is_empty())
    {
        return Err("kernel flow context did not match test traffic".into());
    }
    let (programs, maps) = hooks.ids()?;
    hooks.test_policy(Some(v4.port()))?;
    drop(hooks);
    child(helper, &group.path, v4, false)?;
    let active = loaded_programs()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| context("verify program removal", e))?;
    if active.iter().any(|info| programs.contains(&info.id())) {
        return Err("owned BPF program remained after detach".into());
    }
    // Program/map reclamation is RCU-deferred. Verify disappearance with a bounded wait.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let mut removed = true;
        for id in &maps {
            match MapInfo::from_id(*id) {
                Err(MapError::SyscallError(error))
                    if error.io_error.raw_os_error() == Some(libc::ENOENT) => {}
                Ok(_) => removed = false,
                Err(error) => return Err(context("verify map removal", error)),
            }
        }
        if removed {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err("owned BPF map remained after RCU cleanup deadline".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    group.remove()?;
    Ok(LinuxEbpfSelfTest {
        scope: "isolated_connect_hooks".into(),
        kernel_release: std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .map_err(|e| context("kernel release", e))?
            .trim()
            .into(),
        abi_version: ABI_VERSION,
        object_sha256: format!("{:x}", Sha256::digest(&object)),
        programs,
        maps,
        stats,
        observations,
        map_update_enforced: true,
        connectivity_restored_after_detach: true,
        programs_removed: true,
        maps_removed: true,
        cgroup_removed: !group.path.exists(),
        datapath_ready: false,
    })
}
