#!/usr/bin/env python3
"""Run the production read-only collector in an owned, networkless Rust container.

This exercises the exact collector source with generated DTO declarations, not the
whole daemon/API or any loaded eBPF datapath. The report states that limited scope.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", required=True, help="already installed official/local Rust image")
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    source = (ROOT / "crates/domain/src/topology.rs").read_text().split(
        "#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct LinuxDiagnostics"
    )[0]
    source = source.replace("use serde::{Deserialize, Serialize};", "").replace(
        ", Serialize, Deserialize", ""
    )
    harness = 'extern crate self as hps_domain;\n' + source + '''
#[path="/source/crates/platform/src/topology.rs"] mod topology;
fn main() {
 let first=topology::topology();let second=topology::topology();
 assert!(first.supported && first.kernel_release.is_some());
 assert!(first.network_namespace.is_some() && first.cgroup_namespace.is_some());
 assert!(!first.interfaces.is_empty());
 assert_eq!(first.network_namespace,second.network_namespace);
 assert_eq!(first.interfaces.len(),second.interfaces.len());
 assert!(first.interfaces.iter().all(|v|v.name!="bonding_masters"));
 assert!(first.routes.len()<=256);
 println!("{{\\"scope\\":\\"real_linux_read_only_production_collector\\",\\"kernel\\":\\"{}\\",\\"interfaces\\":{},\\"routes\\":{},\\"mounts\\":{},\\"stable_namespace\\":true,\\"network_configuration_writes\\":false,\\"new_bpf_loads\\":false,\\"full_daemon_api_or_datapath_verified\\":false}}",first.kernel_release.unwrap(),first.interfaces.len(),first.routes.len(),first.mounts.len());
}
'''
    with tempfile.TemporaryDirectory(prefix="hps-issue25-topology-") as temporary:
        Path(temporary, "collector.rs").write_text(harness)
        result = subprocess.run([
            "docker", "run", "--rm", "--pull", "never", "--network", "none",
            "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--tmpfs", "/tmp:rw,nosuid,nodev,exec,size=64m",
            "--mount", f"type=bind,src={ROOT},dst=/source,readonly",
            "--mount", f"type=bind,src={temporary},dst=/validation,readonly",
            args.image, "sh", "-c",
            "rustc --edition 2024 /validation/collector.rs -o /tmp/collector && /tmp/collector",
        ], check=False, capture_output=True, text=True, timeout=60)
        if result.returncode:
            raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    report = json.loads(result.stdout)
    report["container_scope"] = "new_owned_networkless_readonly_capabilityless_container_removed_after_run"
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
