#!/usr/bin/env python3
"""Validate actual CLI/serial output; never manufactures a kernel report."""
import argparse
import ipaddress
import json
import pathlib
parser = argparse.ArgumentParser()
parser.add_argument("report", type=pathlib.Path)
parser.add_argument("--kernel-prefix", required=True)
args = parser.parse_args()
reports = []
for line in args.report.read_text().splitlines():
    try:
        value = json.loads(line)
        if value.get("scope") == "isolated_connect_hooks":
            reports.append(value)
    except (json.JSONDecodeError, AttributeError):
        continue
assert len(reports) == 1, "expected exactly one actual kernel report"
v = reports[0]
assert v["kernel_release"].startswith(args.kernel_prefix)
assert v["abi_version"] == 1 and not v["datapath_ready"]
for key in ("map_update_enforced", "connectivity_restored_after_detach", "programs_removed", "maps_removed", "cgroup_removed"):
    assert v[key], key
assert len(v["programs"]) == 2 and len(set(v["programs"])) == 2
assert len(v["maps"]) >= 3
stats = {s["family"]: s for s in v["stats"]}
assert stats["ipv4"]["connects"] >= 3 and stats["ipv4"]["allowed"] >= 2 and stats["ipv4"]["denied"] >= 1
assert stats["ipv6"]["allowed"] >= 1
assert all(s["errors"] == 0 for s in v["stats"])
assert {o["family"] for o in v["observations"]} == {"ipv4", "ipv6"}
assert len({o["socket_cookie"] for o in v["observations"]}) >= 4
for observation in v["observations"]:
    assert observation["process_id"] > 0 and observation["thread_id"] > 0
    assert observation["cgroup_id"] > 0 and observation["protocol"] == 6
    assert observation["command"] and ipaddress.ip_address(observation["destination"]).is_loopback
    assert 0 < observation["port"] < 65536
print(json.dumps({"verified": True, "kernel": v["kernel_release"], "object_sha256": v["object_sha256"], "program_ids": v["programs"], "map_ids": v["maps"]}))
