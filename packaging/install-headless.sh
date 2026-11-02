#!/bin/sh
set -eu
umask 077
hps_package=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
hps_prefix=${HPS_INSTALL_PREFIX:-/usr/local}
hps_config_dir=${HPS_INSTALL_CONFIG_DIR:-}
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix) hps_prefix=${2:?missing prefix}; shift 2 ;;
        --config-dir) hps_config_dir=${2:?missing config directory}; shift 2 ;;
        *) echo 'Usage: install.sh [--prefix /absolute/path] [--config-dir /absolute/path]' >&2; exit 2 ;;
    esac
done
case "$hps_prefix" in /*) ;; *) echo 'prefix must be absolute' >&2; exit 2;; esac
case "$hps_config_dir" in ''|/*) ;; *) echo 'config directory must be absolute' >&2; exit 2;; esac
cd "$hps_package"
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum --check SHA256SUMS >/dev/null
else
    shasum -a 256 -c SHA256SUMS >/dev/null
fi
mkdir -p "$hps_prefix/bin"
hps_stage=$(mktemp -d "$hps_prefix/bin/.hps-install.XXXXXX")
trap 'rm -rf "$hps_stage"' EXIT HUP INT TERM
install -m 755 bin/hps "$hps_stage/hps"
install -m 755 bin/hpsd "$hps_stage/hpsd"
# Wrong architecture or a damaged binary fails before either installed image changes.
"$hps_stage/hps" --version >/dev/null
"$hps_stage/hpsd" --build-info >/dev/null
for hps_name in hps hpsd; do
    if [ -d "$hps_prefix/bin/$hps_name" ]; then
        echo 'binary destination is a directory; installation refused' >&2
        exit 2
    fi
done
mv -f "$hps_stage/hps" "$hps_prefix/bin/hps"
mv -f "$hps_stage/hpsd" "$hps_prefix/bin/hpsd"
if [ -f share/ebpf/hps-ebpf.elf ]; then
    mkdir -p "$hps_prefix/share/hps/ebpf"
    install -m 644 share/ebpf/hps-ebpf.elf "$hps_prefix/share/hps/ebpf/hps-ebpf.elf"
    install -m 644 share/ebpf/acceptance.json "$hps_prefix/share/hps/ebpf/acceptance.json"
fi
if [ -n "$hps_config_dir" ]; then
    mkdir -p "$hps_config_dir"
    if [ ! -e "$hps_config_dir/hps.toml" ]; then
        install -m 600 config/hps.toml "$hps_config_dir/hps.toml"
    fi
fi
printf 'Installed hps and hpsd in %s/bin\n' "$hps_prefix"
printf 'Use hps service --local to install a supervised service explicitly.\n'
