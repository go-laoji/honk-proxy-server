#!/usr/bin/env python3
"""Build native CI downloads, separate from the verified eBPF release packages."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "target/ci-build"
TARGETS = {
    "x86_64-pc-windows-msvc": ("windows", ".exe", "nsis"),
    "x86_64-unknown-linux-gnu": ("linux", "", "deb"),
    "aarch64-unknown-linux-gnu": ("linux", "", "deb"),
    "aarch64-apple-darwin": ("macos", "", "dmg"),
    "x86_64-apple-darwin": ("macos", "", "dmg"),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def verify_web(directory):
    web = json.loads((directory / "web-manifest.json").read_text(encoding="utf-8"))
    pages = json.loads((ROOT / "web/console/pages.manifest.json").read_text(encoding="utf-8"))
    require(web["pages"] == [page["file"] for page in pages], "shared Web pages mismatch")
    for relative, expected in web["assets"].items():
        path = PurePosixPath(relative)
        require(not path.is_absolute() and ".." not in path.parts and "\\" not in relative and ":" not in relative, "unsafe Web asset path")
        asset = directory / relative
        require(asset.resolve().is_relative_to(directory.resolve()), "Web asset outside dist")
        require(digest(asset) == expected, "shared Web checksum mismatch")
    require(all(page in web["assets"] for page in web["pages"]), "shared Web page missing")
    canonical = json.dumps(web["assets"], ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    require(hashlib.sha256(canonical).hexdigest() == web["web_build_id"], "shared Web build ID mismatch")
    return web


def metadata(binary, name, target, revision, web, environment):
    result = subprocess.run([str(binary), "build-info" if name == "hps" else "--build-info"],
                            env=environment, capture_output=True, text=True, check=True, timeout=30)
    info = json.loads(result.stdout)
    require(info["format_version"] == 1 and info["binary"] == name, "unexpected binary metadata")
    require(info["source_revision"] == revision and info["source_dirty"] is False, "binary source is stale or dirty")
    require(info["architecture"] == target.split("-")[0] and info["platform"] == TARGETS[target][0], "binary target mismatch")
    if name != "hps":
        require(info["version"]["web_build_id"] == web["web_build_id"], "binary embeds a different Web artifact")
    return info


def manifest(package, target, revision, web, binaries):
    return {
        "format_version": 1,
        "package": package,
        "artifact_kind": "ci-build",
        "release_qualified": False,
        "source_revision": revision,
        "target": target,
        "web_build_id": web["web_build_id"],
        "web_pages": web["pages"],
        "binaries": binaries,
        "ebpf_object_included": False,
        "ebpf_kernel_acceptance_performed": False,
        "cloud_signed_or_notarized": False,
        "windows_persistent_service_supported": False,
        "toolchains": json.loads((ROOT / "packaging/toolchains.json").read_text(encoding="utf-8")),
    }


def checksums(directory):
    files = sorted(path for path in directory.rglob("*") if path.is_file() and path.name != "SHA256SUMS")
    (directory / "SHA256SUMS").write_text("".join(
        f"{digest(path)}  {path.relative_to(directory).as_posix()}\n" for path in files), encoding="utf-8")


def archive_headless(stage, destination, windows):
    if windows:
        archive = destination / (stage.name + ".zip")
        with zipfile.ZipFile(archive, "x", compression=zipfile.ZIP_DEFLATED) as output:
            for path in sorted(stage.rglob("*")):
                if path.is_file():
                    output.write(path, (Path(stage.name) / path.relative_to(stage)).as_posix())
    else:
        archive = destination / (stage.name + ".tar.gz")
        with tarfile.open(archive, "x:gz") as output:
            output.add(stage, arcname=stage.name)
    archive.with_name(archive.name + ".sha256").write_text(
        f"{digest(archive)}  {archive.name}\n", encoding="utf-8")
    return archive


def headless(target, revision, web, environment, release, destination):
    subprocess.run(["cargo", "build", "--locked", "--release", "--target", target,
                    "-p", "hps-cli", "-p", "hps-daemon"], cwd=ROOT, env=environment, check=True)
    suffix = TARGETS[target][1]
    binaries = {name: metadata(release / (name + suffix), name, target, revision, web, environment)
                for name in ("hps", "hpsd")}
    require(binaries["hps"]["version"]["version"] == binaries["hpsd"]["version"]["version"], "CLI/Daemon version mismatch")
    version = binaries["hpsd"]["version"]["version"]
    with tempfile.TemporaryDirectory(prefix="hps-ci-package-") as temporary:
        stage = Path(temporary) / f"hps-headless-{version}-{target}-ci"
        (stage / "bin").mkdir(parents=True)
        for name in binaries:
            output = stage / "bin" / (name + suffix)
            shutil.copyfile(release / output.name, output)
            output.chmod(0o755)
        (stage / "config").mkdir()
        shutil.copyfile(ROOT / "packaging/templates/hps.toml", stage / "config/hps.toml")
        shutil.copyfile(ROOT / "docs/ci-builds-usage.md", stage / "README.md")
        write_json(stage / "web-manifest.json", web)
        report = manifest("headless", target, revision, web, binaries)
        write_json(stage / "manifest.json", report)
        checksums(stage)
        archive_headless(stage, destination, target.endswith("windows-msvc"))
    write_json(destination / "manifest.json", report)


def desktop(target, revision, web, environment, release, destination):
    platform, suffix, bundle = TARGETS[target]
    daemon = release / ("hpsd" + suffix)
    daemon_info = metadata(daemon, "hpsd", target, revision, web, environment)
    staging = OUTPUT / "desktop-inputs"
    staging.mkdir(exist_ok=True)
    resource = staging / daemon.name
    shutil.copyfile(daemon, resource)
    resource.chmod(0o755)
    config = {
        "build": {"beforeBuildCommand": "", "frontendDist": str(ROOT / "dist")},
        "bundle": {"resources": {
            str(resource): "bin/" + resource.name,
            str(ROOT / "packaging/templates/hps.toml"): "config/hps.toml",
            str(ROOT / "dist/web-manifest.json"): "web-manifest.json",
        }},
    }
    if platform == "macos":
        config["bundle"]["macOS"] = {"signingIdentity": "-"}
    override = staging / "tauri-ci.json"
    write_json(override, config)
    # Invoke the JS CLI directly so Windows does not need to execute pnpm.cmd.
    cli = ROOT / "node_modules/@tauri-apps/cli/tauri.js"
    subprocess.run(["node", str(cli), "build", "--ci", "--target", target, "--config", str(override),
                    "--bundles", bundle, "--", "--locked"], cwd=ROOT, env=environment, check=True)
    info = metadata(release / ("hps-desktop" + suffix), "hps-desktop", target, revision, web, environment)
    require(info["version"]["version"] == daemon_info["version"]["version"], "Desktop/Daemon version mismatch")
    # Verify the uncompressed bundle's resource before publishing an installer.
    bundle_root = release / "bundle"
    if platform == "macos":
        product = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))["productName"]
        bundled = bundle_root / "macos" / (product + ".app") / "Contents/Resources/bin" / resource.name
        require(digest(bundled) == digest(daemon), "Desktop daemon resource mismatch")
    elif platform == "linux":
        debs = list(bundle_root.glob("deb/*.deb"))
        require(len(debs) == 1, "expected one deb installer")
        with tempfile.TemporaryDirectory(prefix="hps-ci-deb-") as temporary:
            subprocess.run(["dpkg-deb", "--extract", str(debs[0]), temporary], check=True)
            daemons = list(Path(temporary).rglob("bin/hpsd"))
            require(len(daemons) == 1 and digest(daemons[0]) == digest(daemon), "Desktop daemon resource mismatch")
    extension = {"nsis": "exe", "deb": "deb", "dmg": "dmg"}[bundle]
    installers = list(bundle_root.glob(f"{bundle}/*.{extension}"))
    require(len(installers) == 1, "expected one Desktop installer")
    for installer in installers:
        shutil.copyfile(installer, destination / installer.name)
    report = manifest("desktop", target, revision, web, {"hpsd": daemon_info, "desktop": info})
    report["daemon_sha256"] = digest(resource)
    write_json(destination / "manifest.json", report)


def build_environment(revision):
    environment = dict(os.environ, CARGO_TARGET_DIR=str(OUTPUT / "cargo"),
                       HPS_WEB_ASSETS_DIR=str(ROOT / "dist"), HPS_SOURCE_REVISION=revision,
                       HPS_SOURCE_DIRTY="0", HPS_FORBID_HOST_NETWORK_CHANGES="1", CI="true")
    # --ci alone does not disable Tauri's Finder automation during DMG bundling.
    for key in ("TAURI_BUNDLER_DMG_IGNORE_CI", "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID",
                "APPLE_API_ISSUER", "APPLE_API_KEY", "APPLE_API_KEY_PATH", "APPLE_SIGNING_IDENTITY",
                "TAURI_SIGNING_PRIVATE_KEY", "TAURI_SIGNING_PRIVATE_KEY_PASSWORD"):
        environment.pop(key, None)
    return environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", choices=["headless", "desktop"])
    parser.add_argument("--target", choices=TARGETS, required=True)
    args = parser.parse_args()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    require(subprocess.run(["git", "diff", "--quiet", "HEAD", "--"], cwd=ROOT).returncode == 0,
            "commit tracked changes before building CI artifacts")
    host = subprocess.check_output(["rustc", "-vV"], cwd=ROOT, text=True).split("host: ")[1].splitlines()[0]
    require(host == args.target, "CI artifacts must be built and checked on their native target")
    web = verify_web(ROOT / "dist")
    destination = OUTPUT / "artifacts" / args.package
    # Refuse stale outputs; never delete a previous build or the original targets.
    destination.mkdir(parents=True, exist_ok=False)
    environment = build_environment(revision)
    release = OUTPUT / "cargo" / args.target / "release"
    builders = {"headless": headless, "desktop": desktop}
    builders[args.package](args.target, revision, web, environment, release, destination)
    require(verify_web(ROOT / "dist") == web, "shared Web changed during build")
    checksums(destination)
    print(json.dumps({"artifacts": str(destination), "target": args.target, "source_revision": revision}))


if __name__ == "__main__":
    main()
