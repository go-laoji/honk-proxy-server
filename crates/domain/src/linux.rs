use crate::CaptureResourceUsage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureScope {
    pub local_cgroup: Option<String>,
    pub lan_interfaces: Vec<String>,
    pub ipv4: bool,
    pub ipv6: bool,
    pub tcp: bool,
    pub udp: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureListenerStatus {
    pub transport: String,
    pub address: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureHookStatus {
    pub hook: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinuxCaptureStatus {
    pub platform: String,
    pub configured: bool,
    pub state: String,
    pub ready: bool,
    pub reason: Option<String>,
    pub scope: Option<CaptureScope>,
    pub object_path: Option<String>,
    pub object_fingerprint: Option<String>,
    pub userspace_revision: i64,
    pub kernel_revision: Option<i64>,
    pub suspended: bool,
    pub links_present: Option<bool>,
    pub interfaces_ready: Option<bool>,
    pub routes_present: Option<bool>,
    pub listeners: Vec<CaptureListenerStatus>,
    pub program_ids: Vec<u32>,
    pub map_ids: Vec<u32>,
    pub generations: Vec<i64>,
    pub flow_entries: usize,
    pub active_owners: usize,
    pub resources: Option<CaptureResourceUsage>,
    pub rejected_ingress: u64,
    pub hooks: Vec<CaptureHookStatus>,
    pub checked_at_unix_ms: u64,
    pub retry_attempts: u32,
    pub max_retry_attempts: u32,
}
impl LinuxCaptureStatus {
    pub fn unconfigured(revision: i64) -> Self {
        Self {
            platform: std::env::consts::OS.into(),
            configured: false,
            state: if cfg!(target_os = "linux") {
                "unconfigured"
            } else {
                "unsupported"
            }
            .into(),
            ready: false,
            reason: Some(
                if cfg!(target_os = "linux") {
                    "Transparent capture is not configured"
                } else {
                    "Transparent eBPF capture requires Linux"
                }
                .into(),
            ),
            scope: None,
            object_path: None,
            object_fingerprint: None,
            userspace_revision: revision,
            kernel_revision: None,
            suspended: false,
            links_present: None,
            interfaces_ready: None,
            routes_present: None,
            listeners: vec![],
            program_ids: vec![],
            map_ids: vec![],
            generations: vec![],
            flow_entries: 0,
            active_owners: 0,
            resources: None,
            rejected_ingress: 0,
            hooks: vec![],
            checked_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64,
            retry_attempts: 0,
            max_retry_attempts: 0,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxCaptureAction {
    Start,
    Stop,
    Recheck,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxCaptureRequest {
    pub expected_revision: i64,
    pub expected_epoch: String,
    pub action: LinuxCaptureAction,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinuxCaptureOverview<T> {
    pub epoch: String,
    pub running_revision: i64,
    pub desired_revision: i64,
    pub running_configuration: Option<T>,
    pub desired_configuration: Option<T>,
    pub status: LinuxCaptureStatus,
}
