//! Read-only prerequisites plus a separately invoked, owned Linux eBPF lifecycle.
//! Prerequisite observations never prove verifier, attach or forwarding success.
use hps_domain::{CheckStatus, EnvironmentCheck, LinuxPreflight};
use std::io::{self, Read};

trait Source {
    fn read(&self, path: &str, limit: u64) -> io::Result<Vec<u8>>;
}
struct System;
impl Source for System {
    fn read(&self, path: &str, limit: u64) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(limit)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

pub fn preflight() -> LinuxPreflight {
    if cfg!(target_os = "linux") {
        collect(&System)
    } else {
        LinuxPreflight {
            loader_compiled: false,
            platform: std::env::consts::OS.into(),
            kernel_release: None,
            checks: vec![check(
                "platform",
                CheckStatus::Unsupported,
                "eBPF 捕获仅在 Linux 实现；此诊断针对目标 Daemon 而非客户端系统",
            )],
            datapath_ready: false,
            scope: "read_only_prerequisites".into(),
        }
    }
}

fn check(id: &str, status: CheckStatus, detail: impl Into<String>) -> EnvironmentCheck {
    EnvironmentCheck {
        id: id.into(),
        status,
        detail: detail.into(),
    }
}
fn text(source: &impl Source, path: &str) -> Option<String> {
    String::from_utf8(source.read(path, 1_048_576).ok()?).ok()
}
fn mounted<'a>(mounts: &'a str, point: &str, kind: &str) -> Option<&'a str> {
    mounts.lines().find_map(|line| {
        let columns: Vec<_> = line.split_whitespace().collect();
        (columns.len() >= 4 && columns[1] == point && columns[2] == kind).then(|| columns[3])
    })
}

fn collect(source: &impl Source) -> LinuxPreflight {
    let release = text(source, "/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let mut checks = vec![check(
        "kernel",
        if release.is_some() {
            CheckStatus::Pass
        } else {
            CheckStatus::Unknown
        },
        format!(
            "观测内核 {}；最低支持版本将按本项目实际 hook/helper 与验证确定",
            release.as_deref().unwrap_or("未知")
        ),
    )];
    let btf = ["/sys/kernel/btf/vmlinux", "/usr/lib/debug/boot/vmlinux"]
        .into_iter()
        .find(|path| {
            source.read(path, 4).is_ok_and(|bytes| {
                bytes.starts_with(&[0x9f, 0xeb, 1]) || bytes.starts_with(&[0xeb, 0x9f, 1])
            })
        });
    checks.push(check(
        "btf",
        if btf.is_some() {
            CheckStatus::Pass
        } else {
            CheckStatus::Unknown
        },
        btf.map(|path| format!("默认 raw BTF 文件可读：{path}"))
            .unwrap_or_else(|| "默认路径未发现可读 raw BTF；需按所选程序验证 BTF 来源".into()),
    ));
    if let Some(mounts) = text(source, "/proc/mounts") {
        checks.push(check(
            "cgroup_v2",
            if mounted(&mounts, "/sys/fs/cgroup", "cgroup2").is_some() {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            },
            "检查默认路径的实际 cgroup2 挂载；不代表 hook 已可用",
        ));
        let writable = mounted(&mounts, "/sys/fs/bpf", "bpf")
            .is_some_and(|flags| flags.split(',').any(|flag| flag == "rw"));
        checks.push(check(
            "bpffs",
            if writable {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            },
            "默认 pin 路径需要可写 bpffs 挂载；只有目录不算通过，本命令不会自动挂载",
        ));
    } else {
        checks.push(check("cgroup_v2", CheckStatus::Unknown, "无法读取挂载信息"));
        checks.push(check("bpffs", CheckStatus::Unknown, "无法读取挂载信息"));
    }
    if let Some(status) = text(source, "/proc/self/status") {
        let root = status
            .lines()
            .find(|s| s.starts_with("Uid:"))
            .and_then(|s| s.split_whitespace().nth(2))
            .map(|uid| uid == "0");
        checks.push(check(
            "process_uid",
            if root == Some(true) {
                CheckStatus::Pass
            } else {
                CheckStatus::Unknown
            },
            "原型先以 root 验证；UID 不代表具备 BPF/网络能力",
        ));
        let caps = status
            .lines()
            .find(|s| s.starts_with("CapEff:"))
            .and_then(|s| s.split_whitespace().nth(1))
            .and_then(|hex| u64::from_str_radix(hex, 16).ok());
        let basic = caps.map(|caps| caps & (1 << 12) != 0 && (caps & ((1 << 38) | (1 << 21))) != 0);
        checks.push(check("bpf_network_capabilities", match basic { Some(true) => CheckStatus::Pass, Some(false) => CheckStatus::Fail, None => CheckStatus::Unknown },
            "TC 候选路径检查 CAP_NET_ADMIN 以及 CAP_BPF/CAP_SYS_ADMIN；实际加载还受 LSM/seccomp、verifier 与 hook 状态影响"));
    } else {
        checks.push(check(
            "process_uid",
            CheckStatus::Unknown,
            "无法读取进程状态",
        ));
        checks.push(check(
            "bpf_network_capabilities",
            CheckStatus::Unknown,
            "无法读取有效能力",
        ));
    }
    let config = release
        .as_deref()
        .and_then(|release| text(source, &format!("/boot/config-{release}")));
    if let Some(config) = config {
        let options = [
            "CONFIG_BPF",
            "CONFIG_BPF_SYSCALL",
            "CONFIG_NET_SCHED",
            "CONFIG_NET_CLS_ACT",
            "CONFIG_CGROUP_BPF",
        ];
        let missing: Vec<_> = options
            .into_iter()
            .filter(|option| !config.lines().any(|line| line == format!("{option}=y")))
            .collect();
        checks.push(check(
            "kernel_options",
            if missing.is_empty() {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            },
            if missing.is_empty() {
                "TC/cgroup 候选路径的基础选项已启用，最终选项随实际程序验证".into()
            } else {
                format!("基础选项未确认启用：{}", missing.join(", "))
            },
        ));
    } else {
        checks.push(check(
            "kernel_options",
            CheckStatus::Unknown,
            "默认 /boot/config 不可读，不能推断内核选项均已启用",
        ));
    }
    checks.push(check(
        "program_load",
        CheckStatus::Unknown,
        "只读预检未加载/附加 eBPF，未验证原始目标、hook 冲突或代理出口",
    ));
    LinuxPreflight {
        loader_compiled: cfg!(target_os = "linux"),
        platform: "linux".into(),
        kernel_release: release,
        checks,
        datapath_ready: false,
        scope: "read_only_prerequisites".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    struct Fixture(HashMap<String, Vec<u8>>);
    impl Source for Fixture {
        fn read(&self, path: &str, limit: u64) -> io::Result<Vec<u8>> {
            self.0
                .get(path)
                .map(|v| v.iter().copied().take(limit as usize).collect())
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }
    #[test]
    fn root_container_directory_and_new_kernel_do_not_prove_ready() {
        let source = Fixture(HashMap::from([
            (
                "/proc/sys/kernel/osrelease".into(),
                b"7.0.14-linuxkit".to_vec(),
            ),
            ("/sys/kernel/btf/vmlinux".into(), vec![0x9f, 0xeb, 1, 0]),
            (
                "/proc/mounts".into(),
                b"cgroup /sys/fs/cgroup cgroup2 ro,nosuid 0 0\n".to_vec(),
            ),
            (
                "/proc/self/status".into(),
                b"Uid:\t0\t0\t0\t0\nCapEff:\t00000000a80425fb\n".to_vec(),
            ),
        ]));
        let result = collect(&source);
        let status = |id: &str| &result.checks.iter().find(|c| c.id == id).unwrap().status;
        assert_eq!(status("process_uid"), &CheckStatus::Pass);
        assert_eq!(status("cgroup_v2"), &CheckStatus::Pass);
        assert_eq!(status("bpffs"), &CheckStatus::Fail);
        assert_eq!(status("bpf_network_capabilities"), &CheckStatus::Fail);
        assert_eq!(status("program_load"), &CheckStatus::Unknown);
        assert!(!result.datapath_ready);
    }
    #[test]
    fn reference_projects_minimum_kernel_is_not_inherited() {
        let result = collect(&Fixture(HashMap::from([(
            "/proc/sys/kernel/osrelease".into(),
            b"6.6.99".to_vec(),
        )])));
        assert_eq!(result.checks[0].status, CheckStatus::Pass);
        assert!(!result.datapath_ready);
    }
    #[test]
    fn missing_evidence_never_becomes_pass() {
        let result = collect(&Fixture(HashMap::new()));
        assert!(result.checks.iter().all(|c| c.status != CheckStatus::Pass));
        assert!(!result.datapath_ready);
    }
}

#[cfg(target_os = "linux")]
pub mod dns_capture;
#[cfg(target_os = "linux")]
pub mod ebpf;
#[cfg(target_os = "linux")]
mod linux_objects;
#[cfg(target_os = "linux")]
mod rtnetlink;
#[cfg(target_os = "linux")]
pub mod transparent;
#[cfg(not(target_os = "linux"))]
pub mod ebpf {
    use std::{net::SocketAddr, path::Path};
    pub fn self_test(
        _: &Path,
        _: &Path,
        _: &Path,
    ) -> Result<hps_domain::LinuxEbpfSelfTest, String> {
        Err("eBPF self-test requires Linux; no hooks were loaded".into())
    }
    pub fn self_test_child(_: &Path, _: SocketAddr, _: bool) -> Result<(), String> {
        Err("eBPF test helper requires Linux".into())
    }
}

#[cfg(all(test, not(target_os = "linux")))]
mod unsupported_ebpf_tests {
    #[test]
    fn local_self_test_never_claims_success_on_other_platforms() {
        assert!(
            super::ebpf::self_test(
                std::path::Path::new("absent"),
                std::path::Path::new("absent"),
                std::path::Path::new("absent")
            )
            .unwrap_err()
            .contains("requires Linux")
        );
    }
}

pub mod topology;
