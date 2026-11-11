#!/bin/sh
set -eu
hps_repo=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$hps_repo"
if ! git -c safe.directory="$hps_repo" diff --quiet HEAD --; then
    echo 'Commit tracked changes before creating a release build.' >&2
    exit 2
fi
export HPS_SOURCE_REVISION
HPS_SOURCE_REVISION=$(git -c safe.directory="$hps_repo" rev-parse HEAD)
export HPS_SOURCE_DIRTY=0
export HPS_FORBID_HOST_NETWORK_CHANGES=1
export HPS_IP2REGION_DIR="${HPS_IP2REGION_DIR:-${CARGO_TARGET_DIR:-$hps_repo/target}/ip2region}"
python3 -B scripts/fetch-ip2region.py --output "$HPS_IP2REGION_DIR"
# Supply --target explicitly when enabling static musl; proc macros remain host tools.
exec cargo build --locked --release -p hps-cli -p hps-daemon "$@"
