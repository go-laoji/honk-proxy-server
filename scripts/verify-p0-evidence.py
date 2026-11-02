#!/usr/bin/env python3
"""Check traceability, not manufacture a test-pass verdict from file existence."""
import json
from pathlib import Path
import re

root = Path(__file__).resolve().parents[1]
contract = (root / "docs/console-feature-checklist.md").read_text()
expected = set(re.findall(r"^\| ([A-Z]+-\d+) \| P0 \|", contract, re.M))
value = json.loads((root / "docs/validation/p0-evidence.json").read_text())
tasks = {t["id"]: t for t in json.loads((root / "docs/development-tasks.json").read_text())["tasks"]}
seen = set()
for group in value["groups"]:
    task = tasks[group["task"]]
    if task["issue"] != group["issue"] or task["features"] != group["features"]:
        raise ValueError("evidence ownership differs from the development contract")
    for feature in group["features"]:
        if feature in seen:
            raise ValueError("duplicate feature evidence: " + feature)
        seen.add(feature)
    for key in ("implementation", "regression", "evidence"):
        path = root / group[key]
        if not path.is_file() or path.is_symlink() or path.stat().st_size == 0:
            raise ValueError("missing traceability input: " + str(path))
if seen != expected or len(seen) != value["applicable_feature_count"] or len(seen) != 80:
    raise ValueError("80 unique P0 features must have exact task ownership")
kernel = json.loads((root / value["joint_kernel_evidence"]).read_text())
desktop = json.loads((root / value["safe_desktop_evidence"]).read_text())
if not (kernel["full_transparent_datapath_ready"] and kernel["total_actual_kernel_cases"] == 52
        and desktop["host_network_changes_disabled"]
        and not desktop["macos_actual_persistent_proxy_change_verified"]
        and not value["macos_actual_proxy_persistent_change_verified"]
        and not value["paid_supplier_account_call_verified"]
        and not value["unconditional_all_platform_p0_pass"]):
    raise ValueError("preserve actual kernel gates and unverified platform boundaries")
print(json.dumps({"mapped_p0": len(seen), "task_groups": len(value["groups"]), "traceability_valid": True,
                  "unconditional_all_platform_p0_pass": False,
                  "macos_actual_proxy_persistent_change_verified": False,
                  "paid_supplier_account_call_verified": False}))
