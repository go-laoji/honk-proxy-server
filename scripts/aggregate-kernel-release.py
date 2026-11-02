#!/usr/bin/env python3
"""Accept exactly the actual 48+4 case matrix for one byte-identical object."""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--reports", type=Path, required=True)
parser.add_argument("--object", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
digest = hashlib.sha256(args.object.read_bytes()).hexdigest()
reports = []
for arch in ("aarch64", "x86_64"):
    for kernel in ("6.12.115", "7.2.14"):
        for suite, count in (("core", 12), ("nonlocal", 1)):
            path = args.reports / arch / suite / f"{arch}-{kernel}.json"
            value = json.loads(path.read_text())
            log = (path.parent / value["serial_log"]).read_text()
            if not (value["architecture"] == arch and value["actual_kernel"] == kernel
                    and value["passed"] == len(value["cases"]) == count
                    and value["guest_artifacts_sha256"]["hps-ebpf"] == digest
                    and all(f"HPS_CASE_EXIT={case}:0" in log for case in value["cases"])
                    and "HPS_SELF_TEST_EXIT=0" in log):
                raise ValueError(f"incomplete actual-kernel evidence: {path}")
            reports.append({"report": str(path.relative_to(args.reports)), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "passed": count})
args.output.write_text(json.dumps({"full_transparent_datapath_ready": True, "ebpf_abi": 4, "ebpf_object_sha256": digest, "total_actual_kernel_cases": 52, "minimum_verified_kernel": "6.12.115", "architectures": ["aarch64", "x86_64"], "kernels": ["6.12.115", "7.2.14"], "reports": reports}, indent=2) + "\n")
