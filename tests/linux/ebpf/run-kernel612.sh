#!/bin/sh
set -eu
hps_work=$(mktemp -d /tmp/hps-ebpf-XXXXXX)
trap 'rm -rf "$hps_work"' EXIT HUP INT TERM
rootfs="$hps_work/rootfs"
mkdir -p "$rootfs"/bin "$rootfs"/sbin "$rootfs"/lib "$rootfs"/proc "$rootfs"/sys "$rootfs"/dev "$rootfs"/tmp
guest_arch=${HPS_GUEST_ARCH:-aarch64}
case "$guest_arch" in
    aarch64)
        cp /bin/busybox "$rootfs/bin/busybox"
        cp /lib/ld-musl-aarch64.so.1 "$rootfs/lib/ld-musl-aarch64.so.1" ;;
    x86_64)
        cp "${HPS_GUEST_USERLAND:?}/bin/busybox" "$rootfs/bin/busybox"
        cp "$HPS_GUEST_USERLAND/lib/ld-musl-x86_64.so.1" "$rootfs/lib/ld-musl-x86_64.so.1" ;;
    *) exit 2 ;;
esac
for app in sh mount mkdir ip uname cat poweroff modprobe date; do ln -s busybox "$rootfs/bin/$app"; done
# Minimal stable-kernel initrds may not auto-load their RTC driver. Supply the
# real host epoch to this isolated guest before certificate/lease validation.
date -u +%s > "$rootfs/validation-epoch"
cp -a /qemu/kernel/lib/modules "$rootfs/lib/modules"
# The minimal initrd uses BusyBox modprobe. Expand module compression before
# depmod so loading CONFIG_PACKET does not first submit a compressed ELF image.
find "$rootfs/lib/modules" -type f -name '*.ko.gz' -exec gzip -d {} +
find "$rootfs/lib/modules" -type f -name '*.ko.zst' -exec zstd -q -d --rm {} +
kver=$(basename /qemu/kernel/lib/modules/*)
depmod -b "$rootfs" "$kver"
# Keep only the network fixture modules and their actual dependency closure.
# Full stable x86 packages contain unrelated drivers that can fill initramfs.
awk -F ': *' '
function require_module(m, a, n, i) {
    if (seen[m]++) return;
    n=split(dependencies[m], a, / +/);
    for (i=1; i<=n; i++) if (a[i]!="") require_module(a[i]);
}
{ dependencies[$1]=$2; }
END {
    for (m in dependencies) if (m ~ /\/(ipv6|af_packet|veth)\.ko$/) require_module(m);
    for (m in dependencies) if (!(m in seen)) print m;
}' "$rootfs/lib/modules/$kver/modules.dep" > "$hps_work/unneeded-modules"
while IFS= read -r module; do rm "$rootfs/lib/modules/$kver/$module"; done < "$hps_work/unneeded-modules"
depmod -b "$rootfs" "$kver"
if [ -n "${HPS_TEST_MANIFEST:-}" ]; then
    while IFS="$(printf '\t')" read -r binary filter; do
        name="test-$(basename "$binary")"
        if [ ! -f "$rootfs/$name" ]; then
            cp "$binary" "$rootfs/$name"
            if [ "$guest_arch" = aarch64 ]; then strip "$rootfs/$name"; fi
        fi
        printf '/%s\t%s\n' "$name" "$filter" >> "$rootfs/test-cases"
    done < "$HPS_TEST_MANIFEST"
elif [ -n "${HPS_TEST_BINARY:-}" ]; then
    cp "$HPS_TEST_BINARY" "$rootfs/hps"
    printf '%s\n' "${HPS_TEST_FILTER:?HPS_TEST_FILTER is required for an integration test}" > "$rootfs/test-filter"
elif [ -n "${HPS_DNS_TEST:-}" ]; then
    cp "$HPS_DNS_TEST" "$rootfs/hps"
    printf '%s\n' real_dns_capture > "$rootfs/test-filter"
else
    cp "${HPS_CLI:-/artifacts/hps}" "$rootfs/hps"
fi
if [ "$guest_arch" = aarch64 ] && [ -f "$rootfs/hps" ]; then strip "$rootfs/hps"; fi
if [ -n "${HPS_DAEMON_BINARY:-}" ]; then
    cp "$HPS_DAEMON_BINARY" "$rootfs/hpsd"
    if [ "$guest_arch" = aarch64 ]; then strip "$rootfs/hpsd"; fi
fi
cp "${HPS_EBPF_OBJECT:-/artifacts/hps-ebpf}" "$rootfs/hps-ebpf"
if [ -n "${HPS_LEGACY_EBPF_OBJECT:-}" ]; then
    cp "$HPS_LEGACY_EBPF_OBJECT" "$rootfs/hps-ebpf-legacy"
fi
cat > "$rootfs/init" <<'INIT'
#!/bin/sh
export PATH=/bin:/sbin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs dev /dev
mkdir -p /sys/fs/cgroup
mount -t cgroup2 none /sys/fs/cgroup
date -u -s "@$(cat /validation-epoch)" >/dev/null || exit 1
echo "HPS_GUEST_EPOCH=$(date -u +%s)"
modprobe ipv6 || true
modprobe af_packet || true
modprobe veth || true
ip link set lo up
uname -r
if [ -f /hpsd ]; then export HPS_DAEMON_BINARY=/hpsd; fi
if [ -f /test-cases ]; then
    status=0
    while IFS="$(printf '\t')" read -r binary filter; do
        echo "HPS_CASE_START=$filter"
        HPS_TEST_CGROUP_ROOT=/sys/fs/cgroup HPS_EBPF_OBJECT=/hps-ebpf HPS_LEGACY_EBPF_OBJECT=/hps-ebpf-legacy "$binary" --exact "$filter" --ignored --nocapture
        status=$?
        echo "HPS_CASE_EXIT=$filter:$status"
        if [ "$status" -ne 0 ]; then break; fi
    done < /test-cases
elif [ -f /test-filter ]; then
    hps_filter=$(cat /test-filter)
    HPS_TEST_CGROUP_ROOT=/sys/fs/cgroup HPS_EBPF_OBJECT=/hps-ebpf HPS_LEGACY_EBPF_OBJECT=/hps-ebpf-legacy /hps --exact "$hps_filter" --ignored --nocapture
    status=$?
else
    /hps --output json linux self-test --object /hps-ebpf
    status=$?
fi
echo HPS_SELF_TEST_EXIT=$status
poweroff -f
INIT
chmod +x "$rootfs/init"
for artifact in "$rootfs"/test-* "$rootfs/hps" "$rootfs/hpsd" "$rootfs/hps-ebpf" "$rootfs/hps-ebpf-legacy"; do
    if [ -f "$artifact" ]; then
        checksum=$(sha256sum "$artifact")
        printf 'HPS_GUEST_ARTIFACT=%s:%s\n' "$(basename "$artifact")" "${checksum%% *}"
    fi
done
checksum=$(sha256sum /qemu/kernel.apk)
printf 'HPS_KERNEL_APK_SHA256=%s\n' "${checksum%% *}"
(cd "$rootfs" && find . -print0 | cpio --null -o -H newc 2>/dev/null | gzip -1 > "$hps_work/initrd.gz")
kernel=${HPS_GUEST_KERNEL:-/qemu/kernel/boot/vmlinuz-virt}
deadline=${HPS_QEMU_TIMEOUT_SECS:-180}
case "$guest_arch" in
    aarch64)
        timeout -s KILL "$deadline" qemu-system-aarch64 -machine virt,gic-version=3 -accel tcg,thread=multi -cpu max -smp 2 -m 1536 -nic none -kernel "$kernel" -initrd "$hps_work/initrd.gz" -append 'console=ttyAMA0 rdinit=/init loglevel=4 panic=-1' -nographic -no-reboot | tee "$hps_work/serial.log" ;;
    x86_64)
        timeout -s KILL "$deadline" qemu-system-x86_64 -machine q35 -accel tcg,thread=multi -cpu max -smp 2 -m 1536 -nic none -kernel "$kernel" -initrd "$hps_work/initrd.gz" -append 'console=ttyS0 rdinit=/init loglevel=4 panic=-1' -display none -serial stdio -monitor none -no-reboot | tee "$hps_work/serial.log" ;;
esac
grep -q 'HPS_SELF_TEST_EXIT=0' "$hps_work/serial.log"
if [ -f "$rootfs/test-filter" ]; then
    grep -q 'test result: ok. 1 passed;' "$hps_work/serial.log"
fi
if [ -f "$rootfs/test-cases" ]; then
    expected=$(wc -l < "$rootfs/test-cases")
    actual=$(grep -c 'test result: ok. 1 passed;' "$hps_work/serial.log")
    [ "$actual" -eq "$expected" ]
fi
