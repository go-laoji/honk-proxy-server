#!/usr/bin/env python
"""Keep the product's separate manifests and locks on one workspace version (Python 3.11+)."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
STANDALONES = ("src-tauri/Cargo.toml", "ebpf/hps-ebpf/Cargo.toml")
JSON_MANIFESTS = ("package.json", "src-tauri/tauri.conf.json")
LOCKS = ("Cargo.lock", "src-tauri/Cargo.lock", "ebpf/Cargo.lock")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_toml(path):
    return tomllib.loads(path.read_text(encoding="utf-8"))


def valid_version(version):
    require(re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", version),
            "expected a complete SemVer version")
    prerelease = version.split("+", 1)[0].partition("-")[2]
    require(all(not (part.isdecimal() and len(part) > 1 and part.startswith("0"))
                for part in prerelease.split(".")), "numeric prerelease identifiers cannot have leading zeros")
    return version


def product_version(root=ROOT):
    return valid_version(read_toml(root / "Cargo.toml")["workspace"]["package"]["version"])


def version_tag(version):
    # cargo-release's {{version}} excludes build metadata, but includes prerelease.
    return "v" + valid_version(version).split("+", 1)[0]


def workspace_packages(root):
    workspace = read_toml(root / "Cargo.toml")["workspace"]
    packages = {}
    for member in workspace["members"]:
        # This repository deliberately declares explicit member directories.
        require(not any(character in member for character in "*?["), "version check requires explicit workspace members")
        package = read_toml(root / member / "Cargo.toml")["package"]
        require(package.get("version") == {"workspace": True}, f"{member} must inherit the workspace version")
        require(package["name"] not in packages, "duplicate workspace package name")
        packages[package["name"]] = member
    return packages


def check_versions(root=ROOT, tag=None):
    version = product_version(root)
    if tag is not None:
        require(tag == version_tag(version), f"tag {tag!r} does not match product version {version}")
    members = workspace_packages(root)
    names = set(members)
    for relative in STANDALONES:
        package = read_toml(root / relative)["package"]
        require(package["version"] == version, f"{relative} is not version {version}")
        names.add(package["name"])
    for relative in JSON_MANIFESTS:
        require(json.loads((root / relative).read_text(encoding="utf-8"))["version"] == version,
                f"{relative} is not version {version}")
    for relative in LOCKS:
        local = {package["name"]: package["version"]
                 for package in read_toml(root / relative)["package"]
                 if "source" not in package and package["name"] in names}
        require(local and all(value == version for value in local.values()), f"{relative} has stale local package versions")
        if relative == "Cargo.lock":
            require(set(local) == set(members), "root Cargo.lock does not contain all workspace packages")
        elif relative == "src-tauri/Cargo.lock":
            require("hps-desktop" in local, "Desktop lock is missing its package")
        else:
            require({"hps-ebpf", "hps-ebpf-abi"} <= set(local), "eBPF lock is missing its local packages")
    return version


def update_package_version(path, version):
    text = path.read_text(encoding="utf-8")
    # Limit the edit to [package], preserving comments and dependency constraints.
    section = re.search(r"(?ms)^\[package\][^\n]*\n(.*?)(?=^\[|\Z)", text)
    require(section is not None, f"missing [package] in {path}")
    body, count = re.subn(r'(?m)^(version\s*=\s*)"[^"\n]+"',
                          lambda match: match[1] + json.dumps(version), section[1])
    require(count == 1, f"expected one package version in {path}")
    path.write_text(text[:section.start(1)] + body + text[section.end(1):], encoding="utf-8")


def synchronize(root, version):
    require(product_version(root) == version, "cargo-release must bump the workspace version before its hook")
    paths = [root / relative for relative in (*STANDALONES, *JSON_MANIFESTS, *LOCKS)]
    previous = {path: path.read_bytes() for path in paths}
    external = {relative: [package for package in read_toml(root / relative)["package"] if "source" in package]
                for relative in LOCKS}
    try:
        for relative in STANDALONES:
            update_package_version(root / relative, version)
        for relative in JSON_MANIFESTS:
            path = root / relative
            document = json.loads(path.read_text(encoding="utf-8"))
            document["version"] = version
            path.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        environment = dict(os.environ, HPS_FORBID_HOST_NETWORK_CHANGES="1")
        # Cargo refreshes only the local package versions in existing locked graphs.
        # No compilation, dependency upgrade, nightly execution or registry publish.
        for manifest in ("Cargo.toml", "src-tauri/Cargo.toml", "ebpf/Cargo.toml"):
            subprocess.run(["cargo", "update", "--workspace", "--offline",
                            "--manifest-path", str(root / manifest)], cwd=root, env=environment,
                           check=True, stdout=subprocess.DEVNULL)
        check_versions(root)
        require(all([package for package in read_toml(root / relative)["package"] if "source" in package] == packages
                    for relative, packages in external.items()), "version synchronization must not upgrade external dependencies")
    except Exception:
        for path, contents in previous.items():
            path.write_bytes(contents)
        raise


def hook(root=ROOT):
    require(os.environ.get("DRY_RUN") in ("true", "false"), "this hook must be invoked by cargo-release")
    require(os.environ.get("CRATE_NAME") == "hps-cli", "only the representative CLI crate may synchronize versions")
    version = valid_version(os.environ["NEW_VERSION"] +
                            ("+" + os.environ["NEW_METADATA"] if os.environ.get("NEW_METADATA") else ""))
    if os.environ["DRY_RUN"] == "true":
        check_versions(root)
        print(f"Would synchronize Desktop, eBPF, frontend and Cargo locks to {version}; no files changed")
    else:
        synchronize(root, version)
        print(f"Synchronized all product manifests and Cargo locks to {version}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--hook", action="store_true")
    parser.add_argument("--tag", help="require the exact v<product-version> tag")
    args = parser.parse_args()
    if args.hook:
        require(args.tag is None, "--tag belongs to --check")
        hook()
    else:
        print(f"Product versions and Cargo locks agree: {check_versions(tag=args.tag)}")


if __name__ == "__main__":
    main()
