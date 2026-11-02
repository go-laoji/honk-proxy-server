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

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> LinuxDiagnostics {
        let mut capture = crate::LinuxCaptureStatus::unconfigured(1);
        capture.program_ids = vec![7];
        capture.map_ids = vec![9];
        capture.kernel_revision = Some(1);
        capture.checked_at_unix_ms = 100;
        capture.hooks = vec![crate::CaptureHookStatus {
            hook: "ingress".into(),
            observed_sockets: 0,
            marked_sockets: 0,
            captured_packets: 4,
            bypassed_packets: 0,
            rejected_packets: 0,
            unsupported_packets: 0,
            missing_process: 0,
            errors: 0,
            last_error: 0,
            error_phase: 0,
        }];
        LinuxDiagnostics {
            epoch: "same".into(),
            scope: "test".into(),
            capture,
            topology: LinuxTopology {
                supported: true,
                kernel_release: None,
                network_namespace: None,
                cgroup_namespace: None,
                cgroup_membership: None,
                interfaces: vec![],
                routes: vec![],
                mounts: vec![],
                warnings: vec![],
                truncated: false,
            },
        }
    }
    #[test]
    fn changes_only_compare_same_epoch_generation_and_monotonic_counters() {
        let previous = snapshot();
        let mut next = previous.clone();
        next.capture.checked_at_unix_ms = 200;
        next.capture.hooks[0].captured_packets = 8;
        assert_eq!(
            next.counter_delta(&previous).unwrap()["ingress.captured_packets"],
            4
        );
        assert_eq!(next.topology_changed(&previous), Some(false));
        next.capture.hooks[0].captured_packets = 1;
        assert!(next.counter_delta(&previous).is_none());
        next.capture.hooks[0].captured_packets = 8;
        next.capture.map_ids = vec![11];
        assert!(next.counter_delta(&previous).is_none());
        next.capture.map_ids = vec![9];
        next.epoch = "changed".into();
        assert!(next.counter_delta(&previous).is_none());
        assert_eq!(next.topology_changed(&previous), None);
    }
}
