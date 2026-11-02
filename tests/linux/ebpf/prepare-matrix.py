#!/usr/bin/env python3
"""Select exact Cargo test executables; optionally strip copies in the native x86 image."""
import argparse
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--messages", type=Path, required=True)
parser.add_argument("--suite", type=Path, default=Path(__file__).with_name("capture-matrix.json"))
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--strip-x86", action="store_true")
parser.add_argument("--strip-image", default="rust:1.98.1-alpine")
args = parser.parse_args()
artifacts = {}
for line in args.messages.read_text().splitlines():
    value = json.loads(line)
    if value.get("reason") != "compiler-artifact" or not value.get("executable") or not value["profile"]["test"]:
        continue
    package = value["package_id"].rsplit("#", 1)[-1].split("@", 1)[0]
    key = (package, value["target"]["name"])
    executable = value["executable"]
    if key in artifacts and artifacts[key] != executable:
        raise SystemExit(f"ambiguous artifact: {key}")
    artifacts[key] = executable
suite = json.loads(args.suite.read_text())
selected = [(artifacts[(case["package"], case["binary"])], case["test"]) for case in suite]
if args.strip_x86:
    # Use native x86 binutils; generic output formats can erase ELF e_machine.
    binaries = sorted({binary for binary, _ in selected}) + ["/target/x86_64/debug/hpsd"]
    subprocess.run([
        "docker", "run", "--rm", "--platform", "linux/amd64",
        "--name", "hps-matrix-strip-x86", "-v", "hps-validation-target:/target",
        args.strip_image, "sh", "-c",
        'mkdir -p /target/x86-stripped; for binary; do destination="/target/x86-stripped/$(basename "$binary")"; cp "$binary" "$destination" && strip "$destination" || exit; done',
        "sh", *binaries,
    ], check=True)
    selected = [(f"/target/x86-stripped/{Path(binary).name}", case) for binary, case in selected]
args.output.write_text("".join(f"{binary}\t{case}\n" for binary, case in selected))
print(json.dumps({"manifest": str(args.output), "cases": len(selected), "unique_binaries": len({b for b, _ in selected})}))
