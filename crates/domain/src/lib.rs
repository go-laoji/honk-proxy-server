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
