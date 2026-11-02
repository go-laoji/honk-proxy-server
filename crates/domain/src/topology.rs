use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub ifindex: Option<u32>,
    pub mtu: Option<u32>,
    pub state: Option<String>,
    pub master: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRoute {
    pub family: String,
    pub interface: String,
    pub destination: String,
    pub gateway: String,
    pub metric: u32,
    pub flags: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkMount {
    pub path: String,
    pub filesystem: String,
    pub options: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxTopology {
    pub supported: bool,
    pub kernel_release: Option<String>,
    pub network_namespace: Option<String>,
    pub cgroup_namespace: Option<String>,
    pub cgroup_membership: Option<String>,
    pub interfaces: Vec<NetworkInterface>,
    pub routes: Vec<NetworkRoute>,
    pub mounts: Vec<NetworkMount>,
    pub warnings: Vec<String>,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinuxDiagnostics {
    pub epoch: String,
    pub scope: String,
    pub topology: LinuxTopology,
    pub capture: crate::LinuxCaptureStatus,
}
impl LinuxDiagnostics {
    pub fn topology_changed(&self, previous: &Self) -> Option<bool> {
        (self.epoch == previous.epoch).then(|| self.topology != previous.topology)
    }
    /// Counters are comparable only within the same process and exact object generation.
    pub fn counter_delta(
        &self,
        previous: &Self,
    ) -> Option<std::collections::BTreeMap<String, u64>> {
        if self.capture.hooks.is_empty()
            || self.capture.program_ids.is_empty()
            || self.capture.checked_at_unix_ms <= previous.capture.checked_at_unix_ms
            || self.epoch != previous.epoch
            || self.capture.program_ids != previous.capture.program_ids
            || self.capture.map_ids != previous.capture.map_ids
            || self.capture.kernel_revision != previous.capture.kernel_revision
        {
            return None;
        }
        let mut output = std::collections::BTreeMap::new();
        for current in &self.capture.hooks {
            let old = previous
                .capture
                .hooks
                .iter()
                .find(|v| v.hook == current.hook)?;
            for (key, a, b) in [
                (
                    "captured_packets",
                    current.captured_packets,
                    old.captured_packets,
                ),
                (
                    "bypassed_packets",
                    current.bypassed_packets,
                    old.bypassed_packets,
                ),
                (
                    "rejected_packets",
                    current.rejected_packets,
                    old.rejected_packets,
                ),
                ("errors", current.errors, old.errors),
            ] {
                output.insert(format!("{}.{key}", current.hook), a.checked_sub(b)?);
            }
        }
        Some(output)
    }
}
