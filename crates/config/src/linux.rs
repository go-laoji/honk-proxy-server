use crate::ConfigError;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxCaptureSave {
    pub expected_revision: i64,
    pub expected_epoch: String,
    pub configuration: Option<LinuxCaptureConfig>,
}

/// Only explicit scopes are valid. Daemon wiring is added with the real loader;
/// defining this contract does not enable capture on an existing installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LinuxCaptureConfig {
    pub object: PathBuf,
    pub local_cgroup: Option<PathBuf>,
    pub lan_interfaces: Vec<String>,
    pub ipv4: bool,
    pub ipv6: bool,
    pub tcp: bool,
    pub udp: bool,
    pub bypass_cidrs: Vec<String>,
    pub bypass_ports: Vec<u16>,
    pub capture_mark: u32,
    pub bypass_mark: u32,
    pub routing_table: u32,
    pub rule_priority: u32,
    pub tcp_port: u16,
    pub udp_port: u16,
    pub max_flows: u32,
    pub max_pending: u32,
    pub max_udp_sessions: u32,
    pub udp_idle_secs: u32,
    pub max_datagram_bytes: u32,
    pub max_queued_datagrams: u32,
    pub max_queued_bytes: u32,
}
impl Default for LinuxCaptureConfig {
    fn default() -> Self {
        Self {
            object: PathBuf::new(),
            local_cgroup: None,
            lan_interfaces: vec![],
            ipv4: true,
            ipv6: true,
            tcp: true,
            udp: true,
            bypass_cidrs: [
                "0.0.0.0/8",
                "127.0.0.0/8",
                "169.254.0.0/16",
                "224.0.0.0/4",
                "240.0.0.0/4",
                "::/128",
                "::1/128",
                "fe80::/10",
                "ff00::/8",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            bypass_ports: vec![],
            capture_mark: 0x4850_0000,
            bypass_mark: 0x4850_0001,
            routing_table: 10530,
            rule_priority: 31530,
            tcp_port: 18080,
            udp_port: 18080,
            max_flows: 8192,
            max_pending: 128,
            max_udp_sessions: 1024,
            udp_idle_secs: 60,
            max_datagram_bytes: 65507,
            max_queued_datagrams: 256,
            max_queued_bytes: 4 * 1024 * 1024,
        }
    }
}
impl LinuxCaptureConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = || {
            ConfigError::Invalid(
                "invalid Linux capture scope, routing identity or resource bounds".into(),
            )
        };
        if self.object.as_os_str().is_empty()
            || (self.local_cgroup.is_none() && self.lan_interfaces.is_empty())
            || self.local_cgroup.as_ref().is_some_and(|p| !p.is_absolute())
            || self.lan_interfaces.len() > 16
            || (!self.ipv4 && !self.ipv6)
            || (!self.tcp && !self.udp)
            || self.capture_mark == 0
            || self.bypass_mark == 0
            || self.capture_mark == self.bypass_mark
            || !(256..=65535).contains(&self.routing_table)
            || !(1..32766).contains(&self.rule_priority)
            || ((!self.bypass_cidrs.is_empty() || !self.bypass_ports.is_empty())
                && !(2..32765).contains(&self.rule_priority))
            || !(64..=65536).contains(&self.max_flows)
            || !(1..=self.max_flows).contains(&self.max_pending)
            || !(1..=self.max_flows).contains(&self.max_udp_sessions)
            || !(1..=3600).contains(&self.udp_idle_secs)
            || !(512..=65507).contains(&self.max_datagram_bytes)
            || !(1..=4096).contains(&self.max_queued_datagrams)
            || !(self.max_datagram_bytes..=64 * 1024 * 1024).contains(&self.max_queued_bytes)
            || self.bypass_cidrs.len() > 256
            || self.bypass_ports.len() > 256
        {
            return Err(invalid());
        }
        let mut interfaces = HashSet::new();
        for name in &self.lan_interfaces {
            if name.is_empty()
                || name.len() > 15
                || name == "lo"
                || name == "."
                || name == ".."
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
                || !interfaces.insert(name)
            {
                return Err(invalid());
            }
        }
        let mut networks = HashSet::new();
        for cidr in &self.bypass_cidrs {
            let net = cidr.parse::<ipnet::IpNet>().map_err(|_| invalid())?;
            if !networks.insert(net.trunc()) {
                return Err(invalid());
            }
        }
        let mut ports = HashSet::new();
        for port in &self.bypass_ports {
            if *port == 0 || !ports.insert(port) {
                return Err(invalid());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local() -> LinuxCaptureConfig {
        LinuxCaptureConfig {
            object: "/opt/hps/hps-ebpf".into(),
            local_cgroup: Some("/sys/fs/cgroup/proxy-clients".into()),
            ..Default::default()
        }
    }
    #[test]
    fn explicit_capture_scope_is_required_and_local_and_gateway_are_independent() {
        assert!(LinuxCaptureConfig::default().validate().is_err());
        let mut config = local();
        assert!(config.validate().is_ok());
        config.local_cgroup = None;
        config.lan_interfaces = vec!["veth-client".into()];
        assert!(config.validate().is_ok());
        config.local_cgroup = Some("/sys/fs/cgroup/proxy-clients".into());
        assert!(config.validate().is_ok());
        config.lan_interfaces.push("veth-client".into());
        assert!(config.validate().is_err());
        config.lan_interfaces = vec!["lo".into()];
        assert!(config.validate().is_err());
        config.lan_interfaces = vec![];
        config.local_cgroup = Some("relative/cgroup".into());
        assert!(config.validate().is_err());
        assert!(toml::from_str::<LinuxCaptureConfig>("skip_tls_verify=true").is_err());
    }
    #[test]
    fn capture_rejects_routing_collisions_unbounded_queues_and_duplicate_bypasses() {
        let mut config = local();
        config.bypass_mark = config.capture_mark;
        assert!(config.validate().is_err());
        config = local();
        config.routing_table = 254;
        assert!(config.validate().is_err());
        config = local();
        config.rule_priority = 1;
        assert!(config.validate().is_err());
        config.rule_priority = 32765;
        assert!(config.validate().is_err());
        config.rule_priority = 1;
        config.bypass_cidrs.clear();
        assert!(config.validate().is_ok());
        config = local();
        config.max_pending = config.max_flows + 1;
        assert!(config.validate().is_err());
        config = local();
        config.max_queued_bytes = config.max_datagram_bytes - 1;
        assert!(config.validate().is_err());
        config = local();
        config.bypass_cidrs = vec!["192.0.2.1/24".into(), "192.0.2.0/24".into()];
        assert!(config.validate().is_err());
        config = local();
        config.bypass_ports = vec![80, 80];
        assert!(config.validate().is_err());
        config = local();
        config.tcp = false;
        config.udp = false;
        assert!(config.validate().is_err());
        config = local();
        config.ipv4 = false;
        config.ipv6 = false;
        assert!(config.validate().is_err());
        let serialized = toml::to_string(&local()).unwrap();
        assert_eq!(
            toml::from_str::<LinuxCaptureConfig>(&serialized).unwrap(),
            local()
        );
    }
}
