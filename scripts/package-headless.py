#!/usr/bin/env python3
"""Assemble owned build outputs; no service/network configuration or publishing."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def inspect_binary(path, target):
    with path.open("rb") as stream:
        header = stream.read(64)
        arch = target.split("-")[0]
        if "linux" in target:
            require(header[:6] == b'\x7fELF\x02\x01', 'expected 64-bit little-endian Linux ELF')
            require(struct.unpack_from('<H', header, 18)[0] == {'aarch64': 183, 'x86_64': 62}[arch], 'binary architecture mismatch')
            offset = struct.unpack_from("<Q", header, 32)[0]
            size, count = struct.unpack_from("<HH", header, 54)
            require(size >= 56 and count <= 1024, 'invalid ELF program header table')
            for index in range(count):
                stream.seek(offset + index * size)
                kind = struct.unpack("<I", stream.read(4))[0]
                if target.endswith("musl"):
                    require(kind != 3, 'portable musl packages must be static; PT_INTERP found')
        else:
            require(header[:4] == b'\xcf\xfa\xed\xfe', 'expected native 64-bit Mach-O')
            require(struct.unpack_from('<I', header, 4)[0] == {'aarch64': 16777228, 'x86_64': 16777223}[arch], 'binary architecture mismatch')


def build_info(path, daemon):
    result = subprocess.run([str(path), "--build-info" if daemon else "build-info"], capture_output=True, text=True, timeout=20, check=True)
    info = json.loads(result.stdout)
    require(info['format_version'] == 1 and info['binary'] == ('hpsd' if daemon else 'hps'), 'invalid compiled package metadata')
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", required=True, type=Path)
    parser.add_argument("--target", required=True, choices=[arch + suffix for arch in ("aarch64", "x86_64") for suffix in ("-unknown-linux-musl", "-unknown-linux-gnu", "-apple-darwin")])
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/packages")
    parser.add_argument("--web-dir", type=Path, default=ROOT / "dist")
    parser.add_argument("--without-web", action="store_true")
    parser.add_argument("--ebpf", type=Path)
    parser.add_argument("--kernel-report", type=Path, default=ROOT / "docs/validation/linux-transparent-acceptance-2026-11-02.json")
    parser.add_argument("--allow-dirty", action="store_true", help="mark local exploratory artifacts; never treat them as release-ready")
    args = parser.parse_args()
    version = re.search(r'\[workspace.package\]\s*version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text()).group(1)
    revision = subprocess.check_output(["git", "-c", "safe.directory=" + str(ROOT), "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = subprocess.run(["git", "-c", "safe.directory=" + str(ROOT), "diff", "--quiet", "HEAD", "--"], cwd=ROOT).returncode != 0
    require(not dirty or args.allow_dirty, 'tracked working tree is dirty; commit first or explicitly mark an exploratory artifact')
    compiled = args.bin_dir.resolve()
    for name in ("hps", "hpsd"):
        require((compiled / name).is_file(), 'required headless binary missing')
        inspect_binary(compiled / name, args.target)

    web = None if args.without_web else json.loads((args.web_dir / "web-manifest.json").read_text())
    if web is not None:
        pages = json.loads((ROOT / "web/console/pages.manifest.json").read_text())
        require(web['pages'] == [p['file'] for p in pages] and len(pages) == 18, 'invalid compiled package metadata')
        for path, expected in web["assets"].items():
            require(not Path(path).is_absolute() and '..' not in Path(path).parts, 'invalid compiled package metadata')
            require(digest(args.web_dir / path) == expected, 'web artifact checksum mismatch')
        canonical = json.dumps(web["assets"], ensure_ascii=False, separators=(",", ":"), sort_keys=True).encode()
        require(hashlib.sha256(canonical).hexdigest() == web['web_build_id'], 'invalid compiled package metadata')

    native_arch = {"arm64": "aarch64", "AMD64": "x86_64"}.get(platform.machine(), platform.machine())
    native_os = "darwin" if platform.system() == "Darwin" else "linux" if platform.system() == "Linux" else "unsupported"
    can_run = args.target.startswith(native_arch + "-") and native_os in args.target
    require(can_run, "package inside the target architecture, including emulated containers, to verify actual compiled metadata")
    info = None
    if can_run:
        cli_info = build_info(compiled / "hps", False)
        info = build_info(compiled / "hpsd", True)
        require(cli_info['version']['version'] == info['version']['version'] == version, 'invalid compiled package metadata')
        require(info['version']['web_build_id'] == (web['web_build_id'] if web else None), 'daemon embeds a different web artifact')

    kernel = None
    if "linux" in args.target:
        require(args.ebpf and args.ebpf.is_file(), 'Linux packages require the actual verified eBPF object')
        kernel = json.loads(args.kernel_report.read_text())
        require(kernel['full_transparent_datapath_ready'] is True and kernel['total_actual_kernel_cases'] >= 52, 'invalid compiled package metadata')
        require(kernel['ebpf_abi'] == 4 and digest(args.ebpf) == kernel['ebpf_object_sha256'], 'eBPF artifact does not match actual kernel acceptance')
        with args.ebpf.open("rb") as stream:
            header = stream.read(20)
        require(header[:4] == b'\x7fELF' and struct.unpack_from('<H', header, 18)[0] == 247, 'invalid compiled package metadata')

    args.output_dir.mkdir(parents=True, exist_ok=True)
    name = "hps-headless-{}-{}{}".format(version, args.target, "-no-web" if args.without_web else "")
    output = args.output_dir / (name + ".tar.gz")
    require(not output.exists(), 'output artifact already exists; choose a separate output directory')
    with tempfile.TemporaryDirectory(prefix="hps-package-") as temporary:
        stage = Path(temporary) / name
        (stage / "bin").mkdir(parents=True)
        for binary in ("hps", "hpsd"):
            shutil.copyfile(compiled / binary, stage / "bin" / binary)
            (stage / "bin" / binary).chmod(0o755)
        (stage / "config").mkdir()
        shutil.copyfile(ROOT / "packaging/templates/hps.toml", stage / "config/hps.toml")
        (stage / "share/docs").mkdir(parents=True)
        shutil.copyfile(ROOT / "README.md", stage / "share/docs/README.md")
        documentation = subprocess.check_output(["git", "-c", "safe.directory=" + str(ROOT), "ls-files", "docs/*-usage.md"], cwd=ROOT, text=True).splitlines()
        for relative in documentation:
            source=ROOT / relative
            require(source.is_file() and not source.is_symlink(), "documentation must be a tracked regular file")
            shutil.copyfile(source, stage / "share/docs" / source.name)
        shutil.copyfile(ROOT / "packaging/install-headless.sh", stage / "install.sh")
        (stage / "install.sh").chmod(0o755)
        if web:
            (stage / "share/web-manifest.json").write_text(json.dumps(web,indent=2)+"\n")
        if kernel:
            (stage / "share/ebpf").mkdir()
            shutil.copyfile(args.ebpf, stage / "share/ebpf/hps-ebpf.elf")
            shutil.copyfile(args.kernel_report, stage / "share/ebpf/acceptance.json")
            shutil.copyfile(ROOT / "packaging/templates/hpsd.service", stage / "share/hpsd.service")
        manifest = {
            "format_version": 1,
            "package": "headless",
            "version": version,
            "source_revision": revision,
            "dirty_tracked_source": dirty,
            "target": args.target,
            "runtime_metadata_checked": can_run,
            "runtime_metadata": info,
            "web_ui": web is not None,
            "web_build_id": web["web_build_id"] if web else None,
            "web_pages": web["pages"] if web else [],
            "ebpf": {
                "available": bool(kernel),
                "abi": kernel["ebpf_abi"] if kernel else None,
                "sha256": kernel["ebpf_object_sha256"] if kernel else None,
                "minimum_verified_kernel": kernel["minimum_verified_kernel"] if kernel else None,
            },
            "locks": {p: digest(ROOT / p) for p in (
                "Cargo.lock", "src-tauri/Cargo.lock", "ebpf/Cargo.lock", "pnpm-lock.yaml"
            )},
            "toolchains": json.loads((ROOT / "packaging/toolchains.json").read_text()),
            "requires_desktop": False,
            "requires_node_at_runtime": False,
            "macos_actual_proxy_persistent_change_verified": False,
            "paid_supplier_account_call_verified": False,
        }

        (stage / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        files = sorted(p for p in stage.rglob("*") if p.is_file())
        (stage / "SHA256SUMS").write_text("".join("{}  {}\n".format(digest(p), p.relative_to(stage).as_posix()) for p in files))
        epoch = int(subprocess.check_output(["git", "-c", "safe.directory=" + str(ROOT), "show", "-s", "--format=%ct", revision], cwd=ROOT, text=True))
        with output.open("xb") as destination, gzip.GzipFile(fileobj=destination, mode="wb", filename="", mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w") as archive:
            for path in sorted(stage.rglob("*")):
                entry = archive.gettarinfo(str(path), arcname=str(Path(name) / path.relative_to(stage)))
                entry.uid = entry.gid = 0
                entry.uname = entry.gname = ""
                entry.mtime = epoch
                entry.mode = 0o755 if path.is_dir() or path.parent.name == "bin" or path.name == "install.sh" else 0o644
                if path.is_file():
                    with path.open("rb") as contents:
                        archive.addfile(entry, contents)
                else:
                    archive.addfile(entry)
    checksum = digest(output)
    output.with_suffix(output.suffix + ".sha256").write_text(checksum + "  " + output.name + "\n")
    print(json.dumps({"package": str(output), "sha256": checksum, "source_revision": revision, "runtime_checked": can_run, "web_build_id": manifest["web_build_id"]}))


if __name__ == "__main__":
    main()
