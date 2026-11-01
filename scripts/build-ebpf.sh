#!/bin/sh
set -eu
hps_repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if ! command -v bpf-linker >/dev/null 2>&1; then
    echo 'bpf-linker is required; install the official Aya release before building eBPF' >&2
    exit 2
fi
cd "$hps_repo/ebpf"
cargo build --locked --target bpfel-unknown-none -Z build-std=core --release
