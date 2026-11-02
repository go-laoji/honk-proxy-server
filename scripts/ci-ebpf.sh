#!/bin/sh
# Run inside Dockerfile.builder with /work (source) and /out (owned outputs).
set -eu
cd /work
case "$(uname -m)" in
 x86_64) hps_linker_sha=6789717c8cb6a2a5d920f3b6bfac2bd7efc00f6228ecd149108a99699a75c679 ;;
 aarch64) hps_linker_sha=5e201e69dafac7af98f9788f1c68edd5ccc8fa0805093f899bf02bd2947b3935 ;;
 *) exit 2 ;;
esac
mkdir -p /out /tmp/hps-linker
curl --fail --location --retry 2 -o /tmp/hps-linker.tar.zst "https://github.com/aya-rs/bpf-linker/releases/download/v0.12.0/bpf-linker-$(uname -m)-unknown-linux-musl.tar.zst"
printf '%s  %s\n' "$hps_linker_sha" /tmp/hps-linker.tar.zst | sha256sum -c
tar --zstd -xf /tmp/hps-linker.tar.zst -C /tmp/hps-linker
export PATH="/tmp/hps-linker:$PATH"
rustup toolchain install nightly-2026-09-01 --profile minimal --component rust-src
CARGO_TARGET_DIR=/tmp/hps-bpf /work/scripts/build-ebpf.sh
cp /tmp/hps-bpf/bpfel-unknown-none/release/hps-ebpf /out/hps-ebpf.elf
# This immutable project revision really used ABI 3. Keep it only as a rejection
# fixture; never ship it as a working capture object.
mkdir -p /tmp/hps-legacy
git -c safe.directory=/work archive 225fbed3d9810fc6d71048b49b4cefbedfad32a4 | tar -xf - -C /tmp/hps-legacy
cd /tmp/hps-legacy/ebpf
CARGO_TARGET_DIR=/tmp/hps-legacy-target cargo build --locked --target bpfel-unknown-none -Z build-std=core --release
cp /tmp/hps-legacy-target/bpfel-unknown-none/release/hps-ebpf /out/hps-ebpf-abi3.elf
sha256sum /out/*.elf > /out/objects.sha256
