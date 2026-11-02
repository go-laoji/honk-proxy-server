//! Bounded file reads only; never loads BPF, enters a namespace, or changes routes.
use hps_domain::*;
use std::{io::Read, path::Path};
fn read(path: impl AsRef<Path>) -> Option<String> {
    let mut out = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(131073)
        .read_to_string(&mut out)
        .ok()?;
    (out.len() <= 131072).then_some(out)
}
fn link(path: &str) -> Option<String> {
    std::fs::read_link(path)
        .ok()
        .map(|v| v.to_string_lossy().into_owned())
}
fn v4(hex: &str) -> Option<String> {
    Some(std::net::Ipv4Addr::from(u32::from_str_radix(hex, 16).ok()?.to_le_bytes()).to_string())
}
fn v6(hex: &str) -> Option<String> {
    Some(std::net::Ipv6Addr::from(u128::from_str_radix(hex, 16).ok()?).to_string())
}
pub fn topology() -> LinuxTopology {
    let mut out = LinuxTopology {
        supported: cfg!(target_os = "linux"),
        kernel_release: None,
        network_namespace: None,
        cgroup_namespace: None,
        cgroup_membership: None,
        interfaces: vec![],
        routes: vec![],
        mounts: vec![],
        warnings: vec![],
        truncated: false,
    };
    if !out.supported {
        out.warnings
            .push("Read-only topology requires a Linux daemon".into());
        return out;
    }
    out.kernel_release = read("/proc/sys/kernel/osrelease").map(|v| v.trim().to_owned());
    out.network_namespace = link("/proc/self/ns/net");
    out.cgroup_namespace = link("/proc/self/ns/cgroup");
    out.cgroup_membership = read("/proc/self/cgroup");
    if let Some(devices) = read("/proc/net/dev") {
        let mut names = devices
            .lines()
            .skip(2)
            .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim().to_owned()))
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        out.truncated |= names.len() > 128;
        for name in names.into_iter().take(128) {
            let path = Path::new("/sys/class/net").join(&name);
            out.interfaces.push(NetworkInterface {
                name,
                ifindex: read(path.join("ifindex")).and_then(|v| v.trim().parse().ok()),
                mtu: read(path.join("mtu")).and_then(|v| v.trim().parse().ok()),
                state: read(path.join("operstate")).map(|v| v.trim().to_owned()),
                master: std::fs::read_link(path.join("master"))
                    .ok()
                    .and_then(|v| v.file_name().map(|v| v.to_string_lossy().into_owned())),
            });
        }
    } else {
        out.warnings
            .push("Current namespace interface table unavailable".into());
    }
    if let Some(value) = read("/proc/net/route") {
        out.routes.extend(parse_v4(&value));
    } else {
        out.warnings.push("IPv4 route table unavailable".into());
    }
    if let Some(value) = read("/proc/net/ipv6_route") {
        out.routes.extend(parse_v6(&value));
    } else {
        out.warnings.push("IPv6 route table unavailable".into());
    }
    if out.routes.len() > 256 {
        out.truncated = true;
        out.routes.truncate(256);
    }
    if let Some(value) = read("/proc/self/mounts") {
        for line in value.lines() {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() >= 4 && ["bpf", "cgroup2"].contains(&fields[2]) {
                out.mounts.push(NetworkMount {
                    path: fields[1].into(),
                    filesystem: fields[2].into(),
                    options: fields[3].into(),
                });
            }
        }
    } else {
        out.warnings.push("Mount table unavailable".into());
    }
    if out.mounts.len() > 128 {
        out.truncated = true;
        out.mounts.truncate(128);
    }
    out.warnings.push("Route observation covers this daemon network namespace main proc tables; policy rules and other namespaces are not enumerated. Program/map IDs and counters below belong only to this daemon's already loaded capture; observation does not prove forwarding.".into());
    out
}
fn parse_v4(value: &str) -> Vec<NetworkRoute> {
    value
        .lines()
        .skip(1)
        .filter_map(|line| {
            let p = line.split_whitespace().collect::<Vec<_>>();
            if p.len() < 8 {
                return None;
            }
            let mask = u32::from_str_radix(p[7], 16).ok()?.count_ones();
            Some(NetworkRoute {
                family: "ipv4".into(),
                interface: p[0].into(),
                destination: format!("{}/{}", v4(p[1])?, mask),
                gateway: v4(p[2])?,
                metric: p[6].parse().ok()?,
                flags: p[3].into(),
            })
        })
        .take(257)
        .collect()
}
fn parse_v6(value: &str) -> Vec<NetworkRoute> {
    value
        .lines()
        .filter_map(|line| {
            let p = line.split_whitespace().collect::<Vec<_>>();
            if p.len() < 10 {
                return None;
            }
            Some(NetworkRoute {
                family: "ipv6".into(),
                interface: p[9].into(),
                destination: format!("{}/{}", v6(p[0])?, u32::from_str_radix(p[1], 16).ok()?),
                gateway: v6(p[4])?,
                metric: u32::from_str_radix(p[5], 16).ok()?,
                flags: p[8].into(),
            })
        })
        .take(257)
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_endianness_and_prefixes() {
        let r = parse_v4(
            "Iface Destination Gateway Flags RefCnt Use Metric Mask\neth0 00000000 010011AC 0003 0 0 10 00000000",
        );
        assert_eq!(r[0].gateway, "172.17.0.1");
        assert_eq!(r[0].destination, "0.0.0.0/0");
        let r = parse_v6(
            "00000000000000000000000000000001 80 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000000 00000000 00200001 lo",
        );
        assert_eq!(r[0].destination, "::1/128");
    }
}
