#!/usr/bin/env python3
"""Fetch and verify the upstream XDB pair for a native build."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import tempfile
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
METADATA = "https://api.github.com/repos/lionsoul2014/ip2region/contents/data?ref=master"
RAW = "https://raw.githubusercontent.com/lionsoul2014/ip2region/master/data/"
MAX_XDB_BYTES = 128 * 1024 * 1024
XDB_INDEX_START = 256 + 256 * 256 * 8


def fetch(url, limit):
    with urlopen(Request(url, headers={"User-Agent": "honk-proxy-server"}), timeout=30) as response:
        data = response.read(limit + 1)
    if len(data) > limit:
        raise ValueError("upstream response exceeded its size limit")
    return data


def fetch_pair(output):
    entries = {entry["name"]: entry for entry in json.loads(fetch(METADATA, 64 * 1024))}
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="xdb-", dir=output) as temporary:
        staging = Path(temporary)
        manifest = {}
        for family in (4, 6):
            name = f"ip2region_v{family}.xdb"
            entry = entries[name]
            size = entry["size"]
            if not XDB_INDEX_START < size <= MAX_XDB_BYTES:
                raise ValueError(f"invalid upstream size for {name}")
            data = fetch(RAW + name, size)
            blob = hashlib.sha1(f"blob {size}\0".encode())
            blob.update(data)
            if len(data) != size or blob.hexdigest() != entry["sha"]:
                raise ValueError(f"upstream blob changed during download of {name}; retry the build")
            version, index = struct.unpack_from("<HH", data)
            address_family, pointer_size = struct.unpack_from("<HH", data, 16)
            if (version, index, address_family, pointer_size) != (3, 1, family, 4):
                raise ValueError(f"unsupported XDB format for {name}")
            (staging / name).write_bytes(data)
            manifest[f"ipv{family}"] = {"sha": entry["sha"], "size": size}
        (staging / "manifest.json").write_text(json.dumps(manifest) + "\n", encoding="utf-8")
        version = f"{manifest['ipv4']['sha']}-{manifest['ipv6']['sha']}"
        snapshot = output / version
        try:
            staging.rename(snapshot)
        except OSError:
            if not snapshot.is_dir() or any(
                (staging / name).read_bytes() != (snapshot / name).read_bytes()
                for name in ("ip2region_v4.xdb", "ip2region_v6.xdb", "manifest.json")
            ):
                raise
        with tempfile.NamedTemporaryFile(dir=output, delete=False) as pointer:
            pointer.write((version + "\n").encode())
        Path(pointer.name).replace(output / "current")
    print(f"Fetched ip2region {manifest['ipv4']['sha'][:12]}/{manifest['ipv6']['sha'][:12]}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path,
                        default=os.environ.get("HPS_IP2REGION_DIR", ROOT / "target/ip2region"))
    args = parser.parse_args()
    try:
        fetch_pair(args.output)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"ip2region download failed: {error}\n")
