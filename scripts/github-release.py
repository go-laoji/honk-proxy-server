#!/usr/bin/env python
"""Verify the ten native build artifacts and publish their exact bytes to a tagged GitHub Release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from urllib.parse import quote

from versioning import check_versions, require, version_tag

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "x86_64-pc-windows-msvc": ("windows", ".zip", ".exe"),
    "x86_64-unknown-linux-gnu": ("linux", ".tar.gz", ".deb"),
    "aarch64-unknown-linux-gnu": ("linux", ".tar.gz", ".deb"),
    "aarch64-apple-darwin": ("macos", ".tar.gz", ".dmg"),
    "x86_64-apple-darwin": ("macos", ".tar.gz", ".dmg"),
}


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def verified_files(directory):
    files = {path.name: path for path in directory.iterdir()}
    require(all(path.is_file() and not path.is_symlink() for path in files.values()), "artifact must contain regular files only")
    checked = set()
    for line in files["SHA256SUMS"].read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_.+-]+)", line)
        require(match is not None, "invalid SHA256SUMS entry")
        expected, name = match.groups()
        require(name != "SHA256SUMS" and name not in checked and name in files, "invalid or repeated checksum file")
        require(digest(files[name]) == expected, f"artifact checksum mismatch: {name}")
        checked.add(name)
    require(checked == set(files) - {"SHA256SUMS"}, "artifact has missing or unchecked files")
    return files


def prepare(source, destination, tag, revision, version):
    require(tag == version_tag(version), "tag does not match source version")
    require(re.fullmatch(r"[0-9a-f]{40}", revision), "expected a full source commit")
    folders = {f"{package}-{target}" for target in TARGETS for package in ("headless", "desktop")}
    require({path.name for path in source.iterdir()} == folders, "all ten native artifacts are required")
    selected = {}
    web_id = None
    for target, (platform, headless_suffix, desktop_suffix) in TARGETS.items():
        for package in ("headless", "desktop"):
            artifact = f"{package}-{target}"
            files = verified_files(source / artifact)
            manifest = json.loads(files["manifest.json"].read_text(encoding="utf-8"))
            require(manifest["package"] == package and manifest["target"] == target, "artifact target/package mismatch")
            require(manifest["source_revision"] == revision, "artifact source commit mismatch")
            require(manifest["artifact_kind"] == "ci-build" and manifest["release_qualified"] is False
                    and manifest["ebpf_object_included"] is False, "unexpected artifact qualification")
            current_web = manifest["web_build_id"]
            require(re.fullmatch(r"[0-9a-f]{64}", current_web), "invalid Web Build ID")
            require(web_id is None or current_web == web_id, "native artifacts used different Web builds")
            web_id = current_web
            names = {"hps", "hpsd"} if package == "headless" else {"desktop", "hpsd"}
            require(set(manifest["binaries"]) == names, "artifact binary metadata is incomplete")
            for name, info in manifest["binaries"].items():
                binary_name = "hps-desktop" if name == "desktop" else name
                require(info["binary"] == binary_name and info["format_version"] == 1, "binary identity mismatch")
                require(info["source_revision"] == revision and info["source_dirty"] is False, "binary has stale or dirty source")
                require(info["platform"] == platform and info["architecture"] == target.split("-")[0], "binary target mismatch")
                require(info["version"]["version"] == version, "binary version does not match tag")
                if name != "hps":
                    require(info["version"]["web_build_id"] == web_id, "binary Web Build ID mismatch")
            suffix = headless_suffix if package == "headless" else desktop_suffix
            binaries = [path for name, path in files.items() if name.endswith(suffix)]
            require(len(binaries) == 1, "expected exactly one downloadable package")
            binary = binaries[0]
            expected_files = {"manifest.json", "SHA256SUMS", binary.name}
            mappings = {f"{artifact}-manifest.json": files["manifest.json"], binary.name: binary}
            if package == "headless":
                checksum_name = binary.name + ".sha256"
                expected_files.add(checksum_name)
                require(files[checksum_name].read_text(encoding="utf-8").strip() == f"{digest(binary)}  {binary.name}",
                        "headless archive checksum mismatch")
                mappings[checksum_name] = files[checksum_name]
            require(set(files) == expected_files, "unexpected native artifact contents")
            for name, path in mappings.items():
                require(name not in selected, f"Release asset name collision: {name}")
                selected[name] = path
    destination.mkdir(parents=True, exist_ok=False)
    for name, path in selected.items():
        shutil.copyfile(path, destination / name)
    manifest = {
        "format_version": 1, "tag": tag, "version": version, "source_revision": revision,
        "web_build_id": web_id, "targets": list(TARGETS), "release_qualified": False,
        "ebpf_object_included": False,
        "assets": {name: digest(destination / name) for name in sorted(selected)},
    }
    write_json(destination / "release-manifest.json", manifest)
    (destination / "SHA256SUMS").write_text("".join(
        f"{digest(path)}  {path.name}\n" for path in sorted(destination.iterdir())), encoding="utf-8")
    notes = destination.parent / "release-notes.md"
    notes.write_text(f"# Honk Proxy {tag}\n\n"
                     "Windows x64、Linux x64/ARM64、macOS Intel/Apple Silicon 的 CLI/Daemon 和 Desktop 安装包。\n\n"
                     f"源码提交：`{revision}`。所有平台使用同一 Web 构建；附件附带逐平台清单、总清单和 SHA256SUMS。\n\n"
                     "Linux 包使用 glibc，不包含 eBPF 对象；完整透明接管仍需独立的真实内核验收。"
                     "Windows 安装包未做云签名，macOS 使用 ad-hoc 签名、未公证。"
                     "这些附件的 release_qualified=false 表示未计入完整 eBPF 发行门槛，不影响普通显式代理的构建产物下载。\n",
                     encoding="utf-8")
    return manifest


def gh(*args, check=True):
    return subprocess.run(["gh", *args], capture_output=True, text=True, check=check)


def remote_release(repo, tag):
    result = gh("api", "--include", f"repos/{repo}/releases/tags/{quote(tag, safe='')}", check=False)
    header, separator, body = result.stdout.partition("\n\n")
    status = re.match(r"HTTP/\S+ (\d+)", header)
    require(separator and status is not None, "could not read GitHub Release response")
    if status[1] == "404":
        # The tag endpoint only indexes published releases. List releases with
        # this authenticated client to find a newly created or interrupted draft.
        listed = gh("api", "--paginate", "--slurp", f"repos/{repo}/releases?per_page=100")
        matches = [release for page in json.loads(listed.stdout) for release in page
                   if release["tag_name"] == tag]
        require(len(matches) <= 1, "multiple Releases match the same tag; refusing to choose a draft")
        return matches[0] if matches else None
    require(result.returncode == 0 and status[1] == "200", "GitHub Release lookup failed: " + result.stderr)
    return json.loads(body)


def remote_digest(repo, tag, asset):
    value = asset.get("digest")
    if value and value.startswith("sha256:"):
        return value.removeprefix("sha256:")
    # Older assets may not have an API digest. Read their actual bytes instead.
    with tempfile.TemporaryDirectory(prefix="hps-release-asset-") as temporary:
        gh("release", "download", tag, "--repo", repo, "--pattern", asset["name"], "--dir", temporary)
        return digest(Path(temporary) / asset["name"])


def publish(directory, repo, tag, revision):
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo), "invalid GitHub repository")
    files = verified_files(directory)
    manifest = json.loads(files["release-manifest.json"].read_text(encoding="utf-8"))
    require(manifest["tag"] == tag and manifest["source_revision"] == revision, "Release manifest source/tag mismatch")
    require(tag == version_tag(manifest["version"]), "Release manifest version mismatch")
    expected_names = set(manifest["assets"]) | {"release-manifest.json", "SHA256SUMS"}
    require(set(files) == expected_names, "Release manifest and staged assets differ")
    require(all(digest(files[name]) == value for name, value in manifest["assets"].items()), "Release manifest checksums differ")
    prerelease = "-" in manifest["version"].split("+", 1)[0]
    release = remote_release(repo, tag)
    if release is None:
        arguments = ["release", "create", tag, "--repo", repo, "--verify-tag", "--draft", "--title", tag,
                     "--notes-file", str(directory.parent / "release-notes.md")]
        if prerelease:
            arguments.append("--prerelease")
        gh(*arguments)
        release = remote_release(repo, tag)
    require(release and release["tag_name"] == tag and release["prerelease"] == prerelease,
            "existing Release tag/prerelease state differs")
    existing = {asset["name"]: asset for asset in release["assets"]}
    missing = []
    for name, path in files.items():
        if name in existing:
            require(remote_digest(repo, tag, existing[name]) == digest(path),
                    f"existing Release asset has different bytes; refusing to replace {name}")
        else:
            missing.append(str(path))
    if missing:
        gh("release", "upload", tag, "--repo", repo, *sorted(missing))
    confirmed = remote_release(repo, tag)
    require(confirmed is not None, "Release disappeared after upload")
    uploaded = {asset["name"]: asset for asset in confirmed["assets"]}
    require(all(name in uploaded and remote_digest(repo, tag, uploaded[name]) == digest(path)
                for name, path in files.items()), "uploaded Release assets failed verification")
    if confirmed["draft"]:
        gh("release", "edit", tag, "--repo", repo, "--draft=false")
    print(f"Published {len(files)} verified assets to {repo} {tag}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("prepare", "publish"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--input-dir", type=Path, default=ROOT / "target/release-downloads")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/release-assets")
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY"))
    args = parser.parse_args()
    version = check_versions(tag=args.tag)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    tagged = subprocess.check_output(["git", "rev-parse", args.tag + "^{commit}"], cwd=ROOT, text=True).strip()
    require(tagged == revision, "checked out source does not match release tag")
    if args.operation == "prepare":
        prepare(args.input_dir, args.output_dir, args.tag, revision, version)
    else:
        require(args.repo and os.environ.get("GH_TOKEN"), "Release publishing requires a repository and GH_TOKEN")
        publish(args.output_dir, args.repo, args.tag, revision)


if __name__ == "__main__":
    main()
