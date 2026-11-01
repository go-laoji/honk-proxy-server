#!/usr/bin/env python3
"""Validate raw DNS test evidence; never synthesize successful kernel results."""
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("report", type=Path)
parser.add_argument("--kernel-prefix", required=True)
parser.add_argument("--write-json", type=Path)
args = parser.parse_args()
text = args.report.read_text()
if "HPS_DNS_CAPTURE_EVIDENCE=" in text:
    lines = [line for line in text.splitlines() if line.startswith("HPS_DNS_CAPTURE_EVIDENCE=")]
    assert len(lines) == 1, "expected exactly one real kernel evidence record"
    result = json.loads(lines[0].split("=", 1)[1])
else:
    result = json.loads(text)
assert result["kernel"].startswith(args.kernel_prefix)
for key in ["ipv4_udp_tcp", "ipv6_udp_tcp", "parallel_udp_peer_restoration", "getpeername_restored",
            "programs_removed", "maps_removed", "route_removed", "namespace_ownership_reacquired", "cgroup_removed", "daemon_pid_namespace_exclusion"]:
    assert result[key] is True, f"missing kernel proof: {key}"
assert result["full_transparent_datapath_ready"] is False
capture = result["capture"]
assert capture["attached"] is True
assert capture["scope"] == "cgroup_local_application_dns"
assert len(capture["program_ids"]) == len(set(capture["program_ids"])) == 8
assert len(set(capture["map_ids"])) >= 5
assert capture["peers"] == 4 and capture["allocated_slots"] >= 4
assert capture["netns_cookie"] > 0
assert len(capture["object_sha256"]) == 64
assert {s["family"] for s in capture["stats"]} == {"ipv4", "ipv6"}
for stats in capture["stats"]:
    assert stats["redirected"] >= 4 and stats["restored"] >= 4
    assert stats["errors"] == stats["error_code"] == stats["error_phase"] == 0
if args.write_json:
    args.write_json.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
print(f"validated real DNS capture on {result['kernel']}: {capture['object_sha256']}")
