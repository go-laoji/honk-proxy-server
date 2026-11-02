#!/bin/sh
# Native Linux runner only; all networking and cgroups live inside QEMU guests.
set -eu
hps_repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$hps_repo"
hps_arch=$(uname -m)
case "$hps_arch" in aarch64) hps_platform=linux/arm64;; x86_64) hps_platform=linux/amd64;; *) exit 2;; esac
hps_output=${1:?usage: ci-kernel.sh /absolute/output /absolute/ebpf-inputs}
hps_inputs=${2:?missing object input directory}
mkdir -p "$hps_output"
docker build --platform "$hps_platform" -f packaging/Dockerfile.builder -t hps-ci-builder .
# The volume and container names belong exclusively to this workflow job.
docker volume create hps-validation-target >/dev/null
docker run --rm --cpus 2 --memory 4g -v "$hps_repo:/work:ro" -v hps-validation-target:/target -v "$hps_inputs:/objects:ro" -v "$hps_output:/out" -w /work -e "CARGO_TARGET_DIR=/target/$hps_arch" -e HPS_WEB_ASSETS_DIR=/work/dist hps-ci-builder sh -eu -c '
 mkdir -p /target/t15-ebpf/bpfel-unknown-none/release /target/t15-abi3-reference-20261102 /target/x86-userland/bin /target/x86-userland/lib
 cp /objects/hps-ebpf.elf /target/t15-ebpf/bpfel-unknown-none/release/hps-ebpf
 cp /objects/hps-ebpf-abi3.elf /target/t15-abi3-reference-20261102/hps-ebpf
 cp /bin/busybox /target/x86-userland/bin/
 cp /lib/ld-musl-*.so.1 /target/x86-userland/lib/
 cargo test --locked -p hps-platform --lib -p hps-application --test gateway --test transparent --test capture_monitor --test nonlocal --no-run --message-format=json > /out/tests.jsonl
 cargo test --locked -p hps-daemon --test linux_capture --no-run --message-format=json >> /out/tests.jsonl
 cargo build --locked -p hps-daemon
 python3 -c '"'"'import json,subprocess; paths={v["executable"] for line in open("/out/tests.jsonl") if (v:=json.loads(line)).get("executable")}; [subprocess.run(["strip",p],check=True) for p in paths]'"'"'
 strip "$CARGO_TARGET_DIR/debug/hpsd"
 '
python3 tests/linux/ebpf/prepare-matrix.py --messages "$hps_output/tests.jsonl" --output "$hps_output/core.tsv"
python3 tests/linux/ebpf/prepare-matrix.py --messages "$hps_output/tests.jsonl" --suite tests/linux/ebpf/nonlocal-matrix.json --output "$hps_output/nonlocal.tsv"
for hps_kernel in 6.12.115 7.2.14; do
 if [ "$hps_kernel" = 6.12.115 ]; then
  hps_branch=v3.22; hps_repository=main; hps_package=linux-virt-6.12.115-r0.apk
  if [ "$hps_arch" = aarch64 ]; then hps_image=hps-ebpf-kernel612:dev; else hps_image=hps-ebpf-x86-kernel612:dev; fi
 else
  hps_branch=edge; hps_repository=community; hps_package=linux-stable-7.2.14-r0.apk
  if [ "$hps_arch" = aarch64 ]; then hps_image=hps-ebpf-arm-kernel72:dev; else hps_image=hps-ebpf-x86-kernel72:dev; fi
 fi
 docker build -f tests/linux/ebpf/Dockerfile.matrix --build-arg "HPS_GUEST_ARCH=$hps_arch" --build-arg "HPS_KERNEL_BRANCH=$hps_branch" --build-arg "HPS_KERNEL_REPO=$hps_repository" --build-arg "HPS_KERNEL_PACKAGE=$hps_package" -t "$hps_image" tests/linux/ebpf
done
for hps_suite in core nonlocal; do
 python3 tests/linux/ebpf/run-capture-matrix.py --work "$hps_repo" --arm-manifest "$hps_output/$hps_suite.tsv" --x86-manifest "$hps_output/$hps_suite.tsv" --x86-daemon /target/x86_64/debug/hpsd --architecture "$hps_arch" --output "$hps_output/$hps_suite"
done
