#!/usr/bin/env python3
"""Verify an owned archive and installer, including preservation and corruption."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--archive", required=True, type=Path)
parser.add_argument("--prefix", required=True, type=Path)
parser.add_argument("--report", required=True, type=Path)
args = parser.parse_args()
if args.prefix.exists() or not args.prefix.is_absolute():
    raise ValueError("use a new, absolute, owned test prefix")
expected = args.archive.with_suffix(args.archive.suffix + ".sha256").read_text().split()[0]
if hashlib.sha256(args.archive.read_bytes()).hexdigest() != expected:
    raise ValueError("archive sidecar mismatch")
environment = dict(os.environ, HPS_FORBID_HOST_NETWORK_CHANGES="1")
environment.pop("DISPLAY", None)
environment.pop("WAYLAND_DISPLAY", None)
with tempfile.TemporaryDirectory(prefix="hps-install-check-") as directory:
    directory = Path(directory)
    with tarfile.open(args.archive) as archive:
        for member in archive.getmembers():
            if member.issym() or member.islnk() or not (member.isfile() or member.isdir()) or member.name.startswith("/") or ".." in Path(member.name).parts:
                raise ValueError("unexpected archive entry")
        archive.extractall(directory)
    roots = list(directory.iterdir())
    if len(roots) != 1:
        raise ValueError("archive must have one package directory")
    package = roots[0]
    config = args.prefix / "config"
    command = ["sh", str(package / "install.sh"), "--prefix", str(args.prefix), "--config-dir", str(config)]
    subprocess.run(command, env=environment, check=True, capture_output=True)
    marker = b"# operator configuration must survive\n"
    with (config / "hps.toml").open("ab") as output:
        output.write(marker)
    subprocess.run(command, env=environment, check=True, capture_output=True)
    if not (config / "hps.toml").read_bytes().endswith(marker):
        raise ValueError("installer replaced operator configuration")
    manifest = json.loads((package / "manifest.json").read_text())
    metadata = json.loads(subprocess.check_output([str(args.prefix / "bin/hpsd"), "--build-info"], env=environment))
    if metadata != manifest["runtime_metadata"]:
        raise ValueError("installed binary differs from packaged metadata")
    before = {name: hashlib.sha256((args.prefix / "bin" / name).read_bytes()).hexdigest() for name in ("hps", "hpsd")}
    with (package / "bin/hpsd").open("ab") as output:
        output.write(b"invalid-checksum")
    rejected = subprocess.run(command, env=environment, capture_output=True)
    after = {name: hashlib.sha256((args.prefix / "bin" / name).read_bytes()).hexdigest() for name in ("hps", "hpsd")}
    if rejected.returncode == 0 or before != after:
        raise ValueError("corrupt input changed installed binaries")
    result = {"passed": True, "target": manifest["target"], "source_revision": metadata["source_revision"], "web_build_id": metadata["version"]["web_build_id"], "checksums_verified": True, "config_preserved": True, "corrupt_reinstall_preserves_both_binaries": True, "display_required": False, "node_required": False, "host_network_configuration_written": False, "installed_binary_sha256": before}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))
