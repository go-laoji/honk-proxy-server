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
