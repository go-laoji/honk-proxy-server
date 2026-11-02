//! Owns only this daemon's cgroup/TCX links and maps. Listeners and policy routes
//! must be ready before activate; object loading alone never means forwarding.
#[cfg(test)]
mod generation_tests;
mod metadata;
pub use metadata::CapturedMetadata;
mod routes;
#[cfg(test)]
mod tcp_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod udp_tests;
use aya::{
    Ebpf, EbpfLoader,
    maps::{HashMap, LpmTrie, PerCpuArray, SockMap, lpm_trie::Key},
    programs::{
        CgroupAttachMode, CgroupSock, CgroupSockAddr, LinkOrder, SchedClassifier, TcAttachType,
        tc::TcAttachOptions,
    },
};
use hps_config::LinuxCaptureConfig;
use hps_ebpf_abi::*;
use std::{
    collections::BTreeSet,
    ffi::CString,
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::BorrowedFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    sync::Mutex,
};

const ADDRESS_PROGRAMS: [&str; 4] = [
    "hps_tp_connect4",
    "hps_tp_connect6",
    "hps_tp_send4",
    "hps_tp_send6",
];
fn error(label: &str, error: impl std::error::Error) -> String {
    let mut message = format!("transparent capture {label}: {error}");
    let mut cause = error.source();
    for _ in 0..4 {
        let Some(source) = cause else { break };
        message.push_str(&format!(": {source}"));
        cause = source.source();
    }
    message
}
fn interface_index(name: &str) -> Result<u32, String> {
    let name = CString::new(name).map_err(|_| "invalid interface name")?;
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if index == 0 {
        Err(error("interface", std::io::Error::last_os_error()))
    } else {
        Ok(index)
    }
}
pub struct CaptureSockets<'a> {
    pub tcp4: Option<BorrowedFd<'a>>,
    pub tcp6: Option<BorrowedFd<'a>>,
    pub udp4: Option<BorrowedFd<'a>>,
    pub udp6: Option<BorrowedFd<'a>>,
}
pub struct PreparedCapture {
    object: Vec<u8>,
    pub fingerprint: String,
    pub specification: LinuxCaptureConfig,
}
impl PreparedCapture {
    pub fn read(config: &LinuxCaptureConfig) -> Result<Self, String> {
        config.validate().map_err(|e| error("configuration", e))?;
        if cfg!(target_endian = "big") {
            return Err("transparent capture requires a little-endian Linux target".into());
        }
        let mut specification = config.clone();
        if let Some(cgroup) = &mut specification.local_cgroup {
            *cgroup = super::ebpf::cgroup_v2(cgroup)?;
        }
        for name in &specification.lan_interfaces {
            interface_index(name)?;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&specification.object)
            .map_err(|e| error("object", e))?;
        let metadata = file.metadata().map_err(|e| error("object metadata", e))?;
        if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
            return Err("transparent object must be a regular ELF file up to 16 MiB".into());
        }
        let mut object = vec![];
        file.take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut object)
            .map_err(|e| error("object read", e))?;
        if object.len() > 16 * 1024 * 1024 || !object.starts_with(b"\x7fELF") {
            return Err("transparent capture object is not a bounded ELF file".into());
        }
        use sha2::{Digest, Sha256};
        Ok(Self {
            fingerprint: format!("{:x}", Sha256::digest(&object)),
            object,
            specification,
        })
    }
    pub fn verify(&self) -> Result<(), String> {
        self.prepare_bpf().map(|_| ())
    }
    fn prepare_bpf(&self) -> Result<(Ebpf, CapturePolicy), String> {
        let mut bpf = EbpfLoader::new()
            .map_max_entries("HPS_TP_POLICY", 1)
            .map_max_entries("HPS_TP_PROCESS", self.specification.max_flows)
            .map_max_entries("HPS_TP_MODES", self.specification.max_flows)
            .map_max_entries("HPS_TP_FLOWS", self.specification.max_flows)
            .map_max_entries("HPS_TP_STATS", 3)
            .map_max_entries("HPS_TP_SOCKETS", TP_SOCKET_COUNT)
            .load(&self.object)
            .map_err(|e| error("ELF/maps", e))?;
        match bpf.map("HPS_TP_POLICY") {
            Some(aya::maps::Map::HashMap(map)) => {
                let info = map.info().map_err(|e| error("policy map info", e))?;
                if info.map_flags() != 1 || info.max_entries() != 1 {
                    return Err("capture policy requires a single-entry NO_PREALLOC hash".into());
                }
            }
            _ => return Err("capture policy ABI requires a NO_PREALLOC hash".into()),
        }
        for name in ["HPS_TP_PROCESS", "HPS_TP_FLOWS"] {
            match bpf.map(name) {
                Some(aya::maps::Map::HashMap(map)) => {
                    if map
                        .info()
                        .map_err(|e| error("identity map info", e))?
                        .map_flags()
                        != 1
                    {
                        return Err("capture identities require NO_PREALLOC hashes".into());
                    }
                }
                _ => return Err("capture identity ABI requires insert-only hashes".into()),
            }
        }
        let pidns =
            std::fs::metadata("/proc/self/ns/pid").map_err(|e| error("PID namespace", e))?;
        let policy = CapturePolicy {
            abi_version: TP_ABI_VERSION,
            flags: 0,
            revision: 0,
            netns_cookie: super::linux_objects::netns_cookie()
                .map_err(|e| error("network namespace", e))?,
            pidns_device: pidns.dev(),
            pidns_inode: pidns.ino(),
            excluded_tgid: std::process::id(),
            capture_mark: self.specification.capture_mark,
            bypass_mark: self.specification.bypass_mark,
            reserved: 0,
        };
        HashMap::<_, u32, CapturePolicy>::try_from(
            bpf.map_mut("HPS_TP_POLICY")
                .ok_or("capture policy map missing")?,
        )
        .map_err(|e| error("policy ABI", e))?
        .insert(0, policy, 1)
        .map_err(|e| error("initialize disabled policy", e))?;
        let _: HashMap<_, u64, SocketProcess> = HashMap::try_from(
            bpf.map("HPS_TP_PROCESS")
                .ok_or("capture process map missing")?,
        )
        .map_err(|e| error("process ABI", e))?;
        let _: HashMap<_, u64, u32> =
            HashMap::try_from(bpf.map("HPS_TP_MODES").ok_or("capture mode map missing")?)
                .map_err(|e| error("socket mode ABI", e))?;
        let _: HashMap<_, FlowKey, CapturedFlow> =
            HashMap::try_from(bpf.map("HPS_TP_FLOWS").ok_or("capture flow map missing")?)
                .map_err(|e| error("flow ABI", e))?;
        let _: PerCpuArray<_, CaptureStats> =
            PerCpuArray::try_from(bpf.map("HPS_TP_STATS").ok_or("capture stats missing")?)
                .map_err(|e| error("stats ABI", e))?;
        {
            let mut map = HashMap::<_, u32, u8>::try_from(
                bpf.map_mut("HPS_TP_LAN")
                    .ok_or("capture interface map missing")?,
            )
            .map_err(|e| error("interface ABI", e))?;
            for name in &self.specification.lan_interfaces {
                map.insert(interface_index(name)?, 1, 0)
                    .map_err(|e| error("set LAN interface", e))?;
            }
        }
        for cidr in &self.specification.bypass_cidrs {
            match cidr
                .parse::<ipnet::IpNet>()
                .map_err(|_| "invalid bypass CIDR")?
            {
                ipnet::IpNet::V4(net) => LpmTrie::<_, u32, u8>::try_from(
                    bpf.map_mut("HPS_TP_BYPASS4")
                        .ok_or("IPv4 bypass map missing")?,
                )
                .map_err(|e| error("IPv4 bypass ABI", e))?
                .insert(
                    &Key::new(
                        u32::from(net.prefix_len()),
                        u32::from_ne_bytes(net.network().octets()),
                    ),
                    1,
                    0,
                )
                .map_err(|e| error("IPv4 bypass", e))?,
                ipnet::IpNet::V6(net) => {
                    let bytes = net.network().octets();
                    let words = std::array::from_fn(|i| {
                        u32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
                    });
                    LpmTrie::<_, [u32; 4], u8>::try_from(
                        bpf.map_mut("HPS_TP_BYPASS6")
                            .ok_or("IPv6 bypass map missing")?,
                    )
                    .map_err(|e| error("IPv6 bypass ABI", e))?
                    .insert(&Key::new(u32::from(net.prefix_len()), words), 1, 0)
                    .map_err(|e| error("IPv6 bypass", e))?;
                }
            }
        }
        {
            let mut map = HashMap::<_, u32, u8>::try_from(
                bpf.map_mut("HPS_TP_PORTS")
                    .ok_or("capture bypass ports missing")?,
            )
            .map_err(|e| error("port ABI", e))?;
            for port in &self.specification.bypass_ports {
                map.insert(u32::from(*port), 1, 0)
                    .map_err(|e| error("bypass port", e))?;
            }
        }
        for name in ADDRESS_PROGRAMS {
            let program: &mut CgroupSockAddr = bpf
                .program_mut(name)
                .ok_or("capture cgroup program missing")?
                .try_into()
                .map_err(|e| error("cgroup program type", e))?;
            program.load().map_err(|e| error("cgroup verifier", e))?;
        }
        for name in ["hps_tp_create", "hps_tp_release"] {
            let program: &mut CgroupSock = bpf
                .program_mut(name)
                .ok_or("capture socket lifecycle program missing")?
                .try_into()
                .map_err(|e| error("socket lifecycle program type", e))?;
            program
                .load()
                .map_err(|e| error("socket lifecycle verifier", e))?;
        }
        for name in ["hps_tp_ingress", "hps_tp_egress"] {
            let program: &mut SchedClassifier = bpf
                .program_mut(name)
                .ok_or("capture TCX program missing")?
                .try_into()
                .map_err(|e| error("TCX program type", e))?;
            program.load().map_err(|e| error("TCX verifier", e))?;
        }
        Ok((bpf, policy))
    }
    pub fn load(self, sockets: CaptureSockets<'_>) -> Result<Capture, String> {
        let (mut bpf, policy) = self.prepare_bpf()?;
        let ownership = super::linux_objects::ownership("transparent", policy.netns_cookie)
            .map_err(|e| error("namespace ownership", e))?;
        {
            let mut map = SockMap::try_from(
                bpf.map_mut("HPS_TP_SOCKETS")
                    .ok_or("capture sockets missing")?,
            )
            .map_err(|e| error("socket ABI", e))?;
            for (index, enabled, descriptor) in [
                (
                    TP_SOCKET_TCP4,
                    self.specification.tcp && self.specification.ipv4,
                    sockets.tcp4,
                ),
                (
                    TP_SOCKET_TCP6,
                    self.specification.tcp && self.specification.ipv6,
                    sockets.tcp6,
                ),
                (
                    TP_SOCKET_UDP4,
                    self.specification.udp && self.specification.ipv4,
                    sockets.udp4,
                ),
                (
                    TP_SOCKET_UDP6,
                    self.specification.udp && self.specification.ipv6,
                    sockets.udp6,
                ),
            ] {
                if enabled {
                    map.set(
                        index,
                        &descriptor.ok_or("enabled capture listener descriptor missing")?,
                        0,
                    )
                    .map_err(|e| error("publish listener socket", e))?;
                }
            }
        }
        let cgroup = self
            .specification
            .local_cgroup
            .as_ref()
            .map(File::open)
            .transpose()
            .map_err(|e| error("open cgroup", e))?;
        if let Some(cgroup) = &cgroup {
            for name in ADDRESS_PROGRAMS {
                let program: &mut CgroupSockAddr = bpf
                    .program_mut(name)
                    .unwrap()
                    .try_into()
                    .map_err(|e| error("cgroup program type", e))?;
                program
                    // For bpf_link_create, flags must be zero; the kernel gives
                    // cgroup links multi-program semantics itself (6.12 cgroup.c).
                    .attach(cgroup, CgroupAttachMode::Single)
                    .map_err(|e| error("attach disabled cgroup hook", e))?;
            }
            for name in ["hps_tp_create", "hps_tp_release"] {
                let program: &mut CgroupSock = bpf
                    .program_mut(name)
                    .unwrap()
                    .try_into()
                    .map_err(|e| error("socket lifecycle program type", e))?;
                program
                    .attach(cgroup, CgroupAttachMode::Single)
                    .map_err(|e| error("attach socket lifecycle hook", e))?;
            }
        }
        let mut attachments = vec![];
        if cgroup.is_some() {
            attach(
                &mut bpf,
                "hps_tp_egress",
                "lo",
                TcAttachType::Egress,
                &mut attachments,
            )?;
            attach(
                &mut bpf,
                "hps_tp_ingress",
                "lo",
                TcAttachType::Ingress,
                &mut attachments,
            )?;
        }
        for name in &self.specification.lan_interfaces {
            attach(
                &mut bpf,
                "hps_tp_ingress",
                name,
                TcAttachType::Ingress,
                &mut attachments,
            )?;
        }
        Ok(Capture {
            state: Mutex::new(Some(State {
                bpf,
                policy,
                ownership,
                _cgroup: cgroup,
                routes: None,
            })),
            specification: self.specification,
            fingerprint: self.fingerprint,
            attachments,
        })
    }
}
fn attach(
    bpf: &mut Ebpf,
    name: &str,
    interface: &str,
    direction: TcAttachType,
    attachments: &mut Vec<Attachment>,
) -> Result<(), String> {
    let program: &mut SchedClassifier = bpf
        .program_mut(name)
        .ok_or("TCX program missing")?
        .try_into()
        .map_err(|e| error("TCX program type", e))?;
    program
        .attach_with_options(
            interface,
            direction,
            TcAttachOptions::TcxOrder(LinkOrder::last()),
        )
        .map_err(|e| error("attach disabled TCX link", e))?;
    let id = program.info().map_err(|e| error("program ID", e))?.id();
    attachments.push(Attachment {
        interface: interface.into(),
        direction,
        id,
    });
    Ok(())
}
struct Attachment {
    interface: String,
    direction: TcAttachType,
    id: u32,
}
struct State {
    bpf: Ebpf,
    policy: CapturePolicy,
    ownership: File,
    _cgroup: Option<File>,
    routes: Option<routes::OwnedRoutes>,
}
pub struct Capture {
    state: Mutex<Option<State>>,
    pub specification: LinuxCaptureConfig,
    pub fingerprint: String,
    attachments: Vec<Attachment>,
}
impl Capture {
    pub fn prepare_routes(&self) -> Result<(), String> {
        let mut guard = self.state.lock().unwrap();
        let state = guard.as_mut().ok_or("transparent capture is shut down")?;
        if let Some(routes) = &state.routes {
            return if routes.present()? {
                Ok(())
            } else {
                Err("owned capture routing objects are missing".into())
            };
        }
        state.routes = Some(routes::OwnedRoutes::prepare(
            &self.specification,
            state.policy.netns_cookie,
            &state.ownership,
        )?);
        Ok(())
    }
    pub fn routes_present(&self) -> Result<bool, String> {
        let guard = self.state.lock().unwrap();
        let Some(state) = guard.as_ref() else {
            return Ok(false);
        };
        state
            .routes
            .as_ref()
            .map_or(Ok(false), |routes| routes.present())
    }
    /// Call only after route ownership, listeners and the revision's user-space
    /// snapshot are ready. The non-preallocated hash publishes an immutable RCU
    /// element, so each hook observes a complete old or new policy.
    pub fn activate(&self, revision: i64, require_process: bool) -> Result<(), String> {
        if revision <= 0 {
            return Err("capture revision must be positive".into());
        }
        let mut guard = self.state.lock().unwrap();
        let state = guard.as_mut().ok_or("transparent capture is shut down")?;
        if !state
            .routes
            .as_ref()
            .is_some_and(|routes| routes.present().unwrap_or(false))
        {
            return Err("capture routes are not ready".into());
        }
        let mut next = state.policy;
        next.revision = revision as u64;
        next.flags = TP_ENABLED
            | if self.specification.tcp { TP_TCP } else { 0 }
            | if self.specification.udp { TP_UDP } else { 0 }
            | if self.specification.ipv4 { TP_IPV4 } else { 0 }
            | if self.specification.ipv6 { TP_IPV6 } else { 0 }
            | if require_process {
                TP_REQUIRE_PROCESS
            } else {
                0
            };
        HashMap::<_, u32, CapturePolicy>::try_from(
            state
                .bpf
                .map_mut("HPS_TP_POLICY")
                .ok_or("capture policy missing")?,
        )
        .map_err(|e| error("policy ABI", e))?
        .insert(0, next, 2)
        .map_err(|e| error("publish policy", e))?;
        state.policy = next;
        Ok(())
    }
    pub fn flow(&self, key: FlowKey) -> Result<Option<CapturedFlow>, String> {
        let guard = self.state.lock().unwrap();
        let state = guard.as_ref().ok_or("transparent capture is shut down")?;
        if key.netns_cookie != state.policy.netns_cookie {
            return Err("capture flow belongs to another network namespace".into());
        }
        let map = HashMap::<_, FlowKey, CapturedFlow>::try_from(
            state
                .bpf
                .map("HPS_TP_FLOWS")
                .ok_or("capture flow map missing")?,
        )
        .map_err(|e| error("flow ABI", e))?;
        match map.get(&key, 0) {
            Ok(value) => Ok(Some(value)),
            Err(aya::maps::MapError::KeyNotFound) => Ok(None),
            Err(e) => Err(error("flow lookup", e)),
        }
    }
    /// Bounded metadata used for generation retention and owner garbage
    /// collection. This is not an unbounded packet history.
    pub fn flows(&self) -> Result<Vec<(FlowKey, CapturedFlow)>, String> {
        let guard = self.state.lock().unwrap();
        let state = guard.as_ref().ok_or("transparent capture is shut down")?;
        let map = HashMap::<_, FlowKey, CapturedFlow>::try_from(
            state
                .bpf
                .map("HPS_TP_FLOWS")
                .ok_or("capture flow map missing")?,
        )
        .map_err(|e| error("flow ABI", e))?;
        map.iter()
            .take(self.specification.max_flows as usize)
            .map(|entry| entry.map_err(|e| error("flow scan", e)))
            .collect()
    }
    /// The kernel only inserts absent keys and never replaces an identity.
    /// All user-space removals use this same mutex. Thus a matching claim cannot
    /// change between lookup and remove; timestamp refreshes do not change it.
    /// Cancel and drain the corresponding owner before calling this method.
    pub fn retire_flow(&self, key: FlowKey, expected: CapturedFlow) -> Result<bool, String> {
        let mut guard = self.state.lock().unwrap();
        let state = guard.as_mut().ok_or("transparent capture is shut down")?;
        if key.netns_cookie != state.policy.netns_cookie {
            return Err("capture flow belongs to another network namespace".into());
        }
        let mut map = HashMap::<_, FlowKey, CapturedFlow>::try_from(
            state
                .bpf
                .map_mut("HPS_TP_FLOWS")
                .ok_or("capture flow map missing")?,
        )
        .map_err(|e| error("flow ABI", e))?;
        match map.get(&key, 0) {
            Ok(value) if value.same_identity(&expected) => {
                map.remove(&key).map_err(|e| error("retire flow", e))?;
                Ok(true)
            }
            Ok(_) | Err(aya::maps::MapError::KeyNotFound) => Ok(false),
            Err(e) => Err(error("flow lookup", e)),
        }
    }
    pub fn netns_cookie(&self) -> Result<u64, String> {
        self.state
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.policy.netns_cookie)
            .ok_or_else(|| "transparent capture is shut down".into())
    }
    pub fn program_ids(&self) -> Result<Vec<u32>, String> {
        let guard = self.state.lock().unwrap();
        let state = guard.as_ref().ok_or("transparent capture is shut down")?;
        state
            .bpf
            .programs()
            .filter(|(name, _)| name.starts_with("hps_tp_"))
            .map(|(_, program)| {
                program
                    .info()
                    .map(|info| info.id())
                    .map_err(|e| error("program metadata", e))
            })
            .collect()
    }
    pub fn map_ids(&self) -> Result<Vec<u32>, String> {
        let guard = self.state.lock().unwrap();
        let state = guard.as_ref().ok_or("transparent capture is shut down")?;
        let mut maps = BTreeSet::new();
        for (_, program) in state
            .bpf
            .programs()
            .filter(|(name, _)| name.starts_with("hps_tp_"))
        {
            maps.extend(
                program
                    .info()
                    .map_err(|e| error("program metadata", e))?
                    .map_ids()
                    .map_err(|e| error("map IDs", e))?
                    .unwrap_or_default(),
            );
        }
        Ok(maps.into_iter().collect())
    }
    pub fn links_present(&self) -> Result<bool, String> {
        if self.state.lock().unwrap().is_none() {
            return Ok(false);
        }
        for attachment in &self.attachments {
            let (_, programs) =
                SchedClassifier::query_tcx(&attachment.interface, attachment.direction)
                    .map_err(|e| error("TCX link inspection", e))?;
            if !programs.iter().any(|p| p.id() == attachment.id) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn stats(&self) -> Result<Vec<CaptureStats>, String> {
        let guard = self.state.lock().unwrap();
        let state = guard.as_ref().ok_or("transparent capture is shut down")?;
        let map = PerCpuArray::<_, CaptureStats>::try_from(
            state
                .bpf
                .map("HPS_TP_STATS")
                .ok_or("capture stats missing")?,
        )
        .map_err(|e| error("stats ABI", e))?;
        let mut result = vec![];
        for index in 0..3 {
            let mut sum = CaptureStats::default();
            for value in map
                .get(&index, 0)
                .map_err(|e| error("stats read", e))?
                .iter()
            {
                sum.observed_sockets = sum.observed_sockets.saturating_add(value.observed_sockets);
                sum.marked_sockets = sum.marked_sockets.saturating_add(value.marked_sockets);
                sum.captured_packets = sum.captured_packets.saturating_add(value.captured_packets);
                sum.bypassed_packets = sum.bypassed_packets.saturating_add(value.bypassed_packets);
                sum.rejected_packets = sum.rejected_packets.saturating_add(value.rejected_packets);
                sum.unsupported_packets = sum
                    .unsupported_packets
                    .saturating_add(value.unsupported_packets);
                sum.missing_process = sum.missing_process.saturating_add(value.missing_process);
                sum.errors = sum.errors.saturating_add(value.errors);
                if value.last_error != 0 {
                    sum.last_error = value.last_error;
                    sum.error_phase = value.error_phase;
                }
            }
            result.push(sum);
        }
        Ok(result)
    }
    pub fn shutdown(&self) -> Result<(), String> {
        let mut guard = self.state.lock().unwrap();
        let Some(mut state) = guard.take() else {
            return Ok(());
        };
        // Closing owned links detaches even if the disabled-policy update fails.
        state.policy.flags = 0;
        let result = HashMap::<_, u32, CapturePolicy>::try_from(
            state
                .bpf
                .map_mut("HPS_TP_POLICY")
                .ok_or("capture policy missing")?,
        )
        .map_err(|e| error("policy ABI", e))?
        .insert(0, state.policy, 2)
        .map_err(|e| error("disable policy", e));
        drop(state.bpf);
        let cleanup = state
            .routes
            .take()
            .map(|mut routes| routes.cleanup())
            .transpose();
        drop(state.ownership);
        result.and(cleanup.map(|_| ()))
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
