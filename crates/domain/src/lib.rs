use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    pub service: String,
    pub version: String,
    pub api_version: String,
    pub web_build_id: Option<String>,
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
                    "此构建尚未集成真实 eBPF 数据面，不能接管流量"
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
    pub platform: String,
    pub kernel_release: Option<String>,
    pub checks: Vec<EnvironmentCheck>,
    pub datapath_ready: bool,
    pub scope: String,
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
