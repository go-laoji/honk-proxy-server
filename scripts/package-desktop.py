#!/usr/bin/env python3
"""Build a Desktop bundle from one frozen Web artifact and its matching daemon."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("hps_package_support", ROOT / "scripts/package-headless.py")
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--daemon", type=Path, required=True)
    parser.add_argument("--web-dir", type=Path, default=ROOT / "dist")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target/desktop-release")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/packages-desktop")
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument("--ebpf",type=Path)
    parser.add_argument("--kernel-report",type=Path,default=ROOT/"docs/validation/linux-transparent-acceptance-2026-11-02.json")
    parser.add_argument("--debug", action="store_true", help="exploratory validation bundles only")
    parser.add_argument("--bundles", choices=["app", "deb"], default="app" if platform.system() == "Darwin" else "deb")
    args = parser.parse_args()
    require = support.require
    revision = subprocess.check_output(["git", "-c", "safe.directory=" + str(ROOT), "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = subprocess.run(["git", "-c", "safe.directory=" + str(ROOT), "diff", "--quiet", "HEAD", "--"], cwd=ROOT).returncode != 0
    require(not dirty or args.allow_dirty, "commit tracked changes before building a Desktop candidate")
    require(not args.debug or args.allow_dirty, "debug bundles must be explicitly marked exploratory")
    daemon = args.daemon.resolve()
    info = support.build_info(daemon, True)
    kernel=support.verified_ebpf(args.ebpf,args.kernel_report) if platform.system()=="Linux" else None
    web_directory = args.web_dir.resolve()
    web = json.loads((web_directory / "web-manifest.json").read_text())
    require(info["version"]["web_build_id"] == web["web_build_id"], "daemon and Desktop must consume the same Web artifact")
    if not args.allow_dirty:
        require(info.get("source_revision") == revision and info.get("source_dirty") is False, "daemon source is stale or unknown")
    require(len(web["pages"]) == 18, "expected all shared MPA documents")
    for path, digest in web["assets"].items():
        require(not Path(path).is_absolute() and ".." not in Path(path).parts, "invalid Web artifact path")
        require(support.digest(web_directory / path) == digest, "Web artifact was modified")
    target = subprocess.check_output(["rustc", "-vV"], cwd=ROOT, text=True).split("host: ")[1].splitlines()[0]
    require(info["architecture"] == target.split("-")[0], "bundled daemon architecture mismatch")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    target_directory = args.target_dir.resolve()
    require(target_directory != (ROOT / "src-tauri/target").resolve(), "never overwrite the original Desktop target or bundle")
    staging = args.output_dir.resolve() / "desktop-inputs"
    staging.mkdir(exist_ok=True)
    resource_daemon = staging / "hpsd"
    shutil.copyfile(daemon, resource_daemon)
    resource_daemon.chmod(0o755)
    config = {
        "build": {"beforeBuildCommand": "", "frontendDist": str(web_directory)},
        "bundle": {
            "resources": {
                str(resource_daemon): "bin/hpsd",
                str(ROOT / "packaging/templates/hps.toml"): "config/hps.toml",
                str(web_directory / "web-manifest.json"): "web-manifest.json",
            }
        },
    }
    if platform.system() == "Darwin":
        config["bundle"]["macOS"] = {"signingIdentity": "-"}
    if kernel:
        config["bundle"]["resources"][str(args.ebpf.resolve())]="ebpf/hps-ebpf.elf"
        config["bundle"]["resources"][str(args.kernel_report.resolve())]="ebpf/acceptance.json"
    override = staging / "tauri-package.json"
    override.write_text(json.dumps(config, indent=2) + "\n")
    environment = dict(os.environ, CARGO_TARGET_DIR=str(target_directory), HPS_WEB_ASSETS_DIR=str(web_directory), HPS_SOURCE_REVISION=revision, HPS_SOURCE_DIRTY="1" if dirty else "0", HPS_FORBID_HOST_NETWORK_CHANGES="1")
    # This command creates local candidates. It does not perform cloud signing,
    # notarization, updater publishing, or inherit credentials for those actions.
    for key in ("APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH", "APPLE_SIGNING_IDENTITY", "TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"):
        environment.pop(key, None)
    command = ["pnpm", "tauri", "build", "--ci", "--config", str(override), "--bundles", args.bundles]
    if args.debug:
        command.append("--debug")
    command.extend(["--", "--locked"])
    subprocess.run(command, cwd=ROOT, env=environment, check=True)
    mode = "debug" if args.debug else "release"
    binary = target_directory / mode / "hps-desktop"
    result = subprocess.run([str(binary), "--build-info"], env=environment, capture_output=True, text=True, check=True, timeout=20)
    desktop = json.loads(result.stdout)
    require(desktop["version"]["web_build_id"] == info["version"]["web_build_id"], "compiled Desktop embeds different Web bytes")
    require(desktop["source_revision"] == revision and desktop["source_dirty"] == dirty, "compiled Desktop source metadata mismatch")
    bundles = []
    bundle_root = target_directory / mode / "bundle"
    product = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())["productName"]
    for path in sorted(bundle_root.rglob("*")):
        if args.bundles == "app" and path == bundle_root / "macos" / (product + ".app") and path.is_dir():
            require(support.digest(path / "Contents/Resources/bin/hpsd") == support.digest(resource_daemon), "bundled daemon bytes differ")
            bundles.append({"path": str(path), "type": "app", "resource_daemon_sha256": support.digest(resource_daemon)})
        elif args.bundles == "deb" and path.is_file() and path.suffix == ".deb" and path.name.startswith(product + "_" + info["version"]["version"] + "_"):
            with tempfile.TemporaryDirectory(prefix="hps-deb-inspect-") as extracted:
                subprocess.run(["dpkg-deb", "--extract", str(path), extracted], check=True)
                resources = [candidate for candidate in Path(extracted).rglob("hpsd") if candidate.is_file()]
                require(len(resources) == 1 and support.digest(resources[0]) == support.digest(resource_daemon), "deb must contain exactly the matching daemon resource")
            bundles.append({"path": str(path), "sha256": support.digest(path), "type": path.suffix[1:]})
    require(bundles, "no Desktop installation bundle was produced")
    manifest = {"format_version": 1, "package": "desktop", "source_revision": revision, "dirty_tracked_source": dirty, "exploratory_artifact": args.allow_dirty, "target": target, "profile": mode, "web_build_id": web["web_build_id"], "web_pages": web["pages"], "daemon_sha256": support.digest(resource_daemon), "desktop_metadata": desktop, "bundles": bundles, "cloud_signed_or_notarized": False, "published": False, "macos_actual_proxy_persistent_change_verified": False}
    report = args.output_dir / "desktop-manifest.json"
    report.write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"manifest": str(report), "bundles": bundles, "web_build_id": web["web_build_id"]}))


if __name__ == "__main__":
    main()
