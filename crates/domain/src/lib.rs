use serde::{Deserialize, Serialize};
mod selection;
pub use selection::*;
mod routing;
pub use routing::*;
mod nodes;
pub use nodes::*;
mod connections;
pub use connections::*;
mod linux;
pub use linux::*;
mod tasks;
pub use tasks::*;
mod observability;
pub use observability::*;
mod diagnostics;
pub use diagnostics::*;

pub const API_VERSION: &str = "v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsCaptureStats {
    pub family: String,
    pub redirected: u64,
    pub restored: u64,
    pub errors: u64,
    pub bypassed: u64,
    pub error_code: i64,
    pub error_phase: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsCaptureStatus {
    pub scope: String,
    pub attached: bool,
    pub object_sha256: String,
    pub cgroup: String,
    pub port: u16,
    pub ipv4_alias_prefix: String,
    pub ipv6_alias_prefix: String,
    pub netns_cookie: u64,
    pub program_ids: Vec<u32>,
    pub map_ids: Vec<u32>,
    pub peers: usize,
    pub allocated_slots: u64,
    pub stats: Vec<DnsCaptureStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub service: String,
    pub version: String,
    pub api_version: String,
    pub web_build_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledBuildInfo {
    pub format_version: u32,
    pub binary: String,
    pub architecture: String,
    pub platform: String,
    pub source_revision: Option<String>,
    pub source_dirty: bool,
    pub version: VersionInfo,
}
pub fn compiled_build_info(binary: &str, version: VersionInfo) -> CompiledBuildInfo {
    CompiledBuildInfo {
        format_version: 1,
        binary: binary.into(),
        architecture: std::env::consts::ARCH.into(),
        platform: std::env::consts::OS.into(),
        source_revision: option_env!("HPS_SOURCE_REVISION").map(str::to_owned),
        source_dirty: option_env!("HPS_SOURCE_DIRTY") != Some("0"),
        version,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub instance_id: String,
    pub instance_name: String,
    pub management_ready: bool,
    pub proxy_ready: bool,
    pub engine_state: String,
    pub started_at_unix: u64,
    pub uptime_seconds: u64,
    pub platform: String,
    pub version: VersionInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureCapability {
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EbpfCapability {
    pub platform_supported: bool,
    pub compiled: bool,
    pub configured: bool,
    pub attached: bool,
    pub ready: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub management_api: bool,
    pub requires_authentication: bool,
    pub http_proxy: FeatureCapability,
    pub https_proxy: FeatureCapability,
    pub socks5_proxy: FeatureCapability,
    pub linux_ebpf: EbpfCapability,
}

impl Capabilities {
    pub fn explicit_proxy() -> Self {
        let mut capabilities = Self::foundation();
        let available = FeatureCapability {
            available: true,
            reason: None,
        };
        capabilities.http_proxy = available.clone();
        capabilities.https_proxy = available.clone();
        capabilities.socks5_proxy = available;
        capabilities
    }
    pub fn foundation() -> Self {
        let pending = FeatureCapability {
            available: false,
            reason: Some("代理运行时尚未接入".into()),
        };
        Self {
            management_api: true,
            requires_authentication: false,
            http_proxy: pending.clone(),
            https_proxy: pending.clone(),
            socks5_proxy: pending,
            linux_ebpf: EbpfCapability {
                platform_supported: cfg!(target_os = "linux"),
                compiled: false,
                configured: false,
                attached: false,
                ready: false,
                reason: if cfg!(target_os = "linux") {
                    "透明捕获默认关闭，请在 Linux 页面配置并核验实际运行状态；DNS 捕获单独管理"
                } else {
                    "真实 eBPF 数据面仅支持符合内核要求的 Linux"
                }
                .into(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub request_id: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentCheck {
    pub id: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinuxPreflight {
    #[serde(default)]
    pub loader_compiled: bool,
    pub platform: String,
    pub kernel_release: Option<String>,
    pub checks: Vec<EnvironmentCheck>,
    pub datapath_ready: bool,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureResourceUsage {
    pub pending: usize,
    pub max_pending: usize,
    pub udp_sessions: usize,
    pub max_udp_sessions: usize,
    pub queued_datagrams: usize,
    pub max_queued_datagrams: usize,
    pub queued_bytes: usize,
    pub max_queued_bytes: usize,
    pub dropped_datagrams: u64,
    pub invalid_replies: u64,
    pub closed_udp_sessions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResourceLimits {
    pub max_connections: usize,
    pub bytes_per_second: u64,
    pub buffer_bytes: usize,
    pub handshake_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub max_header_bytes: usize,
    pub min_remaining_lifetime_secs: u64,
}
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_connections: 1024,
            bytes_per_second: 0,
            buffer_bytes: 16 * 1024,
            handshake_timeout_ms: 10_000,
            connect_timeout_ms: 10_000,
            idle_timeout_ms: 60_000,
            max_header_bytes: 32 * 1024,
            min_remaining_lifetime_secs: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateStatus {
    pub not_before_unix: i64,
    pub not_after_unix: i64,
    pub valid_now: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundStatus {
    pub id: String,
    pub protocol: String,
    pub configured_listen: String,
    pub configured_enabled: bool,
    #[serde(default)]
    pub runtime_override: Option<bool>,
    pub listen: Option<String>,
    pub running: bool,
    pub active_connections: usize,
    pub max_connections: usize,
    pub bytes_per_second: u64,
    pub accepted_connections: u64,
    pub rejected_global_limit: u64,
    pub rejected_inbound_limit: u64,
    pub failed_connections: u64,
    pub idle_timeouts: u64,
    pub certificate: Option<CertificateStatus>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatus {
    pub inbounds: Vec<InboundStatus>,
    pub active_connections: usize,
    pub eligible_upstreams: usize,
    pub limits: ResourceLimits,
    pub shutting_down: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigRevision {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub checksum: String,
    pub created_at: i64,
    pub note: String,
    pub applied_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigActivation {
    pub id: i64,
    pub revision: i64,
    pub previous_revision: i64,
    pub state: String,
    pub created_at: i64,
    pub error_code: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDraft {
    pub version: i64,
    pub base_revision: i64,
    pub toml: String,
    pub updated_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigWorkspace {
    pub format_version: u32,
    pub desired_revision: i64,
    pub running_revision: i64,
    pub current_toml: String,
    pub desired_toml: String,
    pub draft: ConfigDraft,
    pub revisions: Vec<ConfigRevision>,
    pub activations: Vec<ConfigActivation>,
    pub pending_restart: Vec<String>,
    pub storage_path: Option<String>,
    pub draining_generations: usize,
    pub startup_overrides: Vec<String>,
    pub management_certificate: Option<CertificateStatus>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigExport {
    pub format_version: u32,
    pub revision: i64,
    pub toml: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChange {
    pub path: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
    pub apply_mode: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDiagnostic {
    pub path: String,
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValidation {
    pub valid: bool,
    pub diagnostics: Vec<ConfigDiagnostic>,
    pub changes: Vec<ConfigChange>,
    pub restart_required: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ConfigInput {
    pub toml: String,
    pub expected_revision: i64,
    #[serde(default)]
    pub expected_draft_version: Option<i64>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub use_draft_credentials: bool,
}
impl std::fmt::Debug for ConfigInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigInput")
            .field("toml", &"[REDACTED]")
            .field("note", &"[REDACTED]")
            .field("expected_revision", &self.expected_revision)
            .field("expected_draft_version", &self.expected_draft_version)
            .field("use_draft_credentials", &self.use_draft_credentials)
            .finish()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigActivate {
    pub revision: i64,
    pub expected_revision: i64,
    #[serde(default)]
    pub defer_until_restart: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigActivationResult {
    pub activation_id: i64,
    pub desired_revision: i64,
    pub running_revision: i64,
    pub state: String,
    pub restart_required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionTask {
    pub id: i64,
    #[serde(default = "manual_trigger")]
    pub trigger: String,
    pub provider_id: String,
    pub request_id: String,
    pub config_fingerprint: String,
    pub state: String,
    pub requested_count: u32,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub added: usize,
    pub rejected: usize,
    pub removed: usize,
    pub quota: Option<u64>,
    pub error_code: Option<String>,
    pub retry_after_secs: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseInfo {
    pub id: i64,
    pub batch_id: i64,
    pub provider_id: String,
    pub config_fingerprint: String,
    pub upstream_id: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub received_at: i64,
    pub expires_at: i64,
    pub expiry_basis: String,
    pub state: String,
    pub region: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionInput {
    pub request_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderView<T> {
    pub configuration: T,
    pub active_leases: usize,
    pub latest_task: Option<ExtractionTask>,
    pub health_state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDetail<T> {
    pub provider: ProviderView<T>,
    pub tasks: Vec<ExtractionTask>,
    pub leases: Vec<LeaseInfo>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedRevision {
    pub expected_revision: i64,
}

fn manual_trigger() -> String {
    "manual".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderControllerStatus {
    pub provider_id: String,
    pub enabled: bool,
    pub mode: String,
    pub min_nodes: u32,
    pub target_nodes: u32,
    pub max_nodes: u32,
    pub current_inventory: u32,
    pub projected_inventory: u32,
    #[serde(default)]
    pub pending_health_nodes: u32,
    pub inflight_nodes: u32,
    pub demand_active: bool,
    pub refilling: bool,
    pub decision: String,
    pub planned_count: u32,
    pub day_started_at: i64,
    pub resets_at: i64,
    pub used_requests: u32,
    pub request_budget: u32,
    pub reserved_nodes: u32,
    pub node_budget: u32,
    pub next_allowed_at: i64,
    pub failure_count: u32,
    pub paused_reason: Option<String>,
    pub last_task_id: Option<i64>,
    pub acknowledged_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerEvaluation {
    pub status: ProviderControllerStatus,
    pub task: Option<ExtractionTask>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthTransport {
    Tcp,
    Udp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpFamily {
    Ipv4,
    Ipv6,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthVerdict {
    Unknown,
    Queued,
    Probing,
    Healthy,
    Degraded,
    Failed,
    Unsupported,
    Expired,
    Skipped,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbePhase {
    TargetResolve,
    EndpointConnect,
    UpstreamTls,
    Authentication,
    ProxyTunnel,
    TargetTls,
    TargetHttp,
    Complete,
    Admission,
    Local,
    Unsupported,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub node_id: String,
    pub identity: String,
    pub profile_fingerprint: String,
    pub transport: HealthTransport,
    pub family: IpFamily,
    pub verdict: HealthVerdict,
    pub phase: ProbePhase,
    pub error_code: Option<String>,
    pub endpoint_reachable: bool,
    pub authenticated: Option<bool>,
    pub tunnel_established: bool,
    pub http_status: Option<u16>,
    pub target_ok: Option<bool>,
    /// Observed only from an explicitly configured plain-IP probe response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_ip: Option<std::net::IpAddr>,
    pub latency_ms: u64,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthProbeInput {
    pub request_id: String,
    #[serde(default)]
    pub node_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthRun {
    pub id: i64,
    pub node_id: String,
    pub identity: String,
    pub profile_fingerprint: String,
    pub state: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub result: Option<ProbeResult>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthTask {
    pub id: i64,
    pub request_id: String,
    pub trigger: String,
    pub state: String,
    pub created_at: i64,
    pub total: u32,
    pub completed: u32,
    pub healthy: u32,
    pub failed: u32,
    pub skipped: u32,
    pub merged: u32,
    pub runs: Vec<HealthRun>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealth {
    pub node_id: String,
    pub provider_id: Option<String>,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub enabled: bool,
    pub remaining_secs: Option<u64>,
    pub eligible: bool,
    pub last_traffic: Option<TrafficFeedback>,
    pub results: Vec<ProbeResult>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthOverview<C> {
    pub running_revision: i64,
    pub configuration: C,
    pub nodes: Vec<NodeHealth>,
    pub next_after: Option<String>,
    pub total_nodes: usize,
    pub queued: u32,
    pub probing: u32,
    pub day_started_at: i64,
    pub resets_at: i64,
    pub used_probes: u32,
    pub budget: u32,
    pub tasks: Vec<HealthTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficFeedback {
    pub node_id: String,
    pub identity: String,
    pub phase: ProbePhase,
    pub error_code: String,
    pub invalidated: bool,
    pub observed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EbpfHookStats {
    pub family: String,
    pub connects: u64,
    pub allowed: u64,
    pub denied: u64,
    pub errors: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EbpfFlowObservation {
    pub socket_cookie: u64,
    pub process_id: u32,
    pub thread_id: u32,
    pub cgroup_id: u64,
    pub family: String,
    pub destination: String,
    pub port: u16,
    pub protocol: u32,
    pub command: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinuxEbpfSelfTest {
    pub scope: String,
    pub kernel_release: String,
    pub abi_version: u32,
    pub object_sha256: String,
    pub programs: Vec<u32>,
    pub maps: Vec<u32>,
    pub stats: Vec<EbpfHookStats>,
    pub observations: Vec<EbpfFlowObservation>,
    pub map_update_enforced: bool,
    pub connectivity_restored_after_detach: bool,
    pub programs_removed: bool,
    pub maps_removed: bool,
    pub cgroup_removed: bool,
    pub datapath_ready: bool,
}
