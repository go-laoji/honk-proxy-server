#!/bin/sh
set -eu
hps_work=$(mktemp -d /tmp/hps-ebpf-XXXXXX)
trap 'rm -rf "$hps_work"' EXIT HUP INT TERM
rootfs="$hps_work/rootfs"
mkdir -p "$rootfs"/bin "$rootfs"/sbin "$rootfs"/lib "$rootfs"/proc "$rootfs"/sys "$rootfs"/dev "$rootfs"/tmp
cp /bin/busybox "$rootfs/bin/busybox"
cp /lib/ld-musl-aarch64.so.1 "$rootfs/lib/ld-musl-aarch64.so.1"
for app in sh mount mkdir ip uname cat poweroff modprobe; do ln -s busybox "$rootfs/bin/$app"; done
cp -a /qemu/kernel/lib/modules "$rootfs/lib/modules"
kver=$(basename /qemu/kernel/lib/modules/*)
depmod -b "$rootfs" "$kver"
if [ -n "${HPS_TEST_BINARY:-}" ]; then
    cp "$HPS_TEST_BINARY" "$rootfs/hps"
    printf '%s\n' "${HPS_TEST_FILTER:?HPS_TEST_FILTER is required for an integration test}" > "$rootfs/test-filter"
elif [ -n "${HPS_DNS_TEST:-}" ]; then
    cp "$HPS_DNS_TEST" "$rootfs/hps"
    printf '%s\n' real_dns_capture > "$rootfs/test-filter"
else
    cp "${HPS_CLI:-/artifacts/hps}" "$rootfs/hps"
fi
strip "$rootfs/hps"
cp "${HPS_EBPF_OBJECT:-/artifacts/hps-ebpf}" "$rootfs/hps-ebpf"
cat > "$rootfs/init" <<'INIT'
#!/bin/sh
export PATH=/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs dev /dev
mkdir -p /sys/fs/cgroup
mount -t cgroup2 none /sys/fs/cgroup
modprobe ipv6 || true
modprobe af_packet || true
ip link set lo up
uname -r
if [ -f /test-filter ]; then
    hps_filter=$(cat /test-filter)
    HPS_TEST_CGROUP_ROOT=/sys/fs/cgroup HPS_EBPF_OBJECT=/hps-ebpf /hps --exact "$hps_filter" --ignored --nocapture
else
    /hps --output json linux self-test --object /hps-ebpf
fi
status=$?
echo HPS_SELF_TEST_EXIT=$status
poweroff -f
INIT
chmod +x "$rootfs/init"
(cd "$rootfs" && find . -print0 | cpio --null -o -H newc 2>/dev/null | gzip -1 > "$hps_work/initrd.gz")
qemu-system-aarch64 -machine virt,gic-version=3 -accel tcg,thread=multi -cpu max -smp 2 -m 1024 -kernel /qemu/kernel/boot/vmlinuz-virt -initrd "$hps_work/initrd.gz" -append 'console=ttyAMA0 rdinit=/init loglevel=4 panic=-1' -nographic -no-reboot | tee "$hps_work/serial.log"
grep -q 'HPS_SELF_TEST_EXIT=0' "$hps_work/serial.log"
