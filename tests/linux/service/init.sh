#!/bin/sh
set -eu
# A fresh mount in Docker's PRIVATE cgroup namespace confines systemd to this
# validation container. Do not bind the host's /sys/fs/cgroup into this test.
mount -t cgroup2 none /sys/fs/cgroup
exec /sbin/init
