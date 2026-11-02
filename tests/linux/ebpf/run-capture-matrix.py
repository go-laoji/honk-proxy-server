#!/usr/bin/env python3
"""Run all capture cases in real QEMU guest kernels, retaining serial evidence."""
import argparse
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--work", type=Path, required=True)
parser.add_argument("--arm-manifest", type=Path, required=True)
parser.add_argument("--x86-manifest", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--architecture", choices=["aarch64", "x86_64"])
parser.add_argument("--kernel", choices=["6.12.115", "7.2.14"])
parser.add_argument("--arm-daemon", default="/target/aarch64/debug/hpsd")
parser.add_argument("--x86-daemon", default="/target/x86-stripped/hpsd")
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
results = []
for arch, kernel, image in [
    ("aarch64", "6.12.115", "hps-ebpf-kernel612:dev"),
    ("x86_64", "6.12.115", "hps-ebpf-x86-kernel612:dev"),
    ("aarch64", "7.2.14", "hps-ebpf-arm-kernel72:dev"),
    ("x86_64", "7.2.14", "hps-ebpf-x86-kernel72:dev"),
]:
    if args.architecture and args.architecture != arch:
        continue
    if args.kernel and args.kernel != kernel:
        continue
    manifest = args.arm_manifest if arch == "aarch64" else args.x86_manifest
    cases = [line.split("\t")[1] for line in manifest.read_text().splitlines()]
    name = f"hps-capture-matrix-{arch}-{kernel.replace('.', '')}"
    log = args.output / f"{arch}-{kernel}.log"
    daemon = args.arm_daemon if arch == "aarch64" else args.x86_daemon
    command = ["docker", "run", "--rm", "--name", name,
        "-v", f"{args.work.resolve()}:/work:ro", "-v", "hps-validation-target:/target:ro",
        "-v", f"{manifest.resolve()}:/matrix.tsv:ro",
        "-e", "HPS_TEST_MANIFEST=/matrix.tsv", "-e", "HPS_QEMU_TIMEOUT_SECS=900",
        "-e", f"HPS_DAEMON_BINARY={daemon}", "-e", f"HPS_GUEST_ARCH={arch}",
        "-e", "HPS_EBPF_OBJECT=/target/t15-ebpf/bpfel-unknown-none/release/hps-ebpf",
        "-e", "HPS_LEGACY_EBPF_OBJECT=/target/t15-abi3-reference-20261102/hps-ebpf"]
    if arch == "x86_64":
        command += ["-e", "HPS_GUEST_USERLAND=/target/x86-userland"]
    if kernel == "7.2.14":
        command += ["-e", "HPS_GUEST_KERNEL=/qemu/kernel/boot/vmlinuz-stable"]
    command += ["--entrypoint", "sh", image, "/work/tests/linux/ebpf/run-kernel612.sh"]
    print(f"START {arch} {kernel} cases={len(cases)} log={log}", flush=True)
    with log.open("w") as output:
        completed = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=1100)
    text = log.read_text()
    exits = [line.removeprefix("HPS_CASE_EXIT=").rsplit(":", 1) for line in text.splitlines() if line.startswith("HPS_CASE_EXIT=")]
    passed = len(re.findall(r"test result: ok\. 1 passed;", text))
    assert completed.returncode == 0, f"guest runner failed; inspect {log}"
    assert exits == [[case, "0"] for case in cases], f"missing or failed case: {log}"
    assert passed == len(cases), f"zero-match or extra test: {log}"
    evidence = [json.loads(line) for line in text.splitlines() if line.startswith("{")]
    assert evidence and all(e.get("kernel", "").startswith(kernel) for e in evidence), f"wrong actual kernel: {log}"
    artifacts = dict(line.removeprefix("HPS_GUEST_ARTIFACT=").split(":", 1) for line in text.splitlines() if line.startswith("HPS_GUEST_ARTIFACT="))
    kernel_digest = [line.removeprefix("HPS_KERNEL_APK_SHA256=") for line in text.splitlines() if line.startswith("HPS_KERNEL_APK_SHA256=")]
    assert len(kernel_digest) == 1 and "hps-ebpf" in artifacts
    result = {"architecture": arch, "actual_kernel": kernel, "image": image, "cases": cases, "passed": passed, "evidence": evidence, "serial_log": log.name, "guest_artifacts_sha256": artifacts, "kernel_apk_sha256": kernel_digest[0], "full_transparent_datapath_ready": False}
    (args.output / f"{arch}-{kernel}.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    results.append(result)
    print(f"PASS {arch} {kernel} cases={passed}", flush=True)
(args.output / "matrix.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
