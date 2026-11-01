use serde::{Deserialize, Serialize};

/// Extra operational restriction; Enabled never overrides configuration or lease admission.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeOperationState {
    #[default]
    Enabled,
    Disabled,
    Draining,
    Deleted,
}
impl NodeOperationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Draining => "draining",
            Self::Deleted => "deleted",
        }
    }
    pub const fn blocking_reason(self) -> Option<&'static str> {
        match self {
            Self::Enabled => None,
            Self::Disabled => Some("manually_disabled"),
            Self::Draining => Some("draining"),
            Self::Deleted => Some("manually_deleted"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeControl {
    pub node_id: String,
    pub state: NodeOperationState,
    pub version: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NodeQuery {
    pub id: Option<String>,
    pub search: Option<String>,
    pub provider: Option<String>,
    pub group: Option<String>,
    pub protocol: Option<String>,
    pub capability: Option<String>,
    pub state: Option<String>,
    pub remaining_max_secs: Option<u64>,
    pub sort: Option<String>,
    pub offset: u32,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeView {
    pub id: String,
    pub kind: String,
    pub provider_id: Option<String>,
    pub lease_id: Option<i64>,
    pub batch_id: Option<i64>,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub configuration_enabled: bool,
    pub has_credentials: bool,
    pub received_at: Option<i64>,
    pub expires_at_unix: Option<u64>,
    pub remaining_secs: Option<u64>,
    pub expiry_basis: Option<String>,
    pub region: Option<String>,
    pub lease_state: String,
    pub operation: NodeControl,
    pub published: bool,
    pub eligible: bool,
    pub admission_reasons: Vec<String>,
    pub tcp_supported: bool,
    pub udp_supported: bool,
    pub health_state: String,
    pub health_results: Vec<crate::ProbeResult>,
    pub groups: Vec<String>,
    pub tags: Vec<String>,
    /// Includes admitted dials and live sockets; health probes are excluded.
    pub active_connections: usize,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeSummary {
    pub inventory: usize,
    pub published: usize,
    pub eligible: usize,
    pub near_expiry: usize,
    pub expired: usize,
    pub restricted: usize,
    pub source_unavailable: usize,
    pub active_connections: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodesOverview {
    pub running_revision: i64,
    pub evaluated_at_unix: i64,
    pub summary: NodeSummary,
    pub inventory_truncated: bool,
    pub providers: Vec<String>,
    pub groups: Vec<String>,
    pub sources: Vec<ProviderNodeSummary>,
    pub filtered_total: usize,
    pub offset: u32,
    pub next_offset: Option<u32>,
    pub nodes: Vec<NodeView>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderNodeSummary {
    pub id: String,
    pub enabled: bool,
    pub refill_enabled: bool,
    pub minimum: u32,
    pub target: u32,
    pub maximum: u32,
    pub inventory: usize,
    pub eligible: usize,
    pub near_expiry: usize,
    pub expired: usize,
    pub latest_task_id: Option<i64>,
    pub latest_task_state: Option<String>,
    pub latest_error_code: Option<String>,
    pub latest_task_at: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeActionInput {
    pub id: String,
    pub action: String,
    pub expected_revision: i64,
    pub expected_control_version: i64,
    pub lease_id: Option<i64>,
    pub batch_id: Option<i64>,
    pub provider_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeActionResult {
    pub pool_refreshed: bool,
    pub running_revision: i64,
    pub control: Option<NodeControl>,
    pub configuration_activation: Option<crate::ConfigActivationResult>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeImportInput {
    pub document: String,
    pub format: String,
    pub default_protocol: String,
    pub expected_revision: i64,
    pub preview_checksum: Option<String>,
    #[serde(default)]
    pub selected_rows: Vec<u32>,
}
impl std::fmt::Debug for NodeImportInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeImportInput")
            .field("document", &"[REDACTED]")
            .field("format", &self.format)
            .field("expected_revision", &self.expected_revision)
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeImportRow {
    pub row: u32,
    pub id: Option<String>,
    pub protocol: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub has_credentials: bool,
    pub valid: bool,
    pub error_code: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeImportPreview {
    pub running_revision: i64,
    pub checksum: String,
    pub rows: Vec<NodeImportRow>,
    pub valid_count: usize,
    pub invalid_count: usize,
    pub selected_count: usize,
    pub applicable: bool,
}
