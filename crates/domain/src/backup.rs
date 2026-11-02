use serde::{Deserialize, Serialize};
/// Full database snapshots contain secrets and are separate from diagnostic exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMetadata {
    pub format_version: u32,
    pub schema_version: i64,
    pub size_bytes: u64,
    pub sha256: String,
    pub verified_at_unix: i64,
    pub contains_credentials: bool,
    pub restore_requires_stopped_daemon: bool,
}
