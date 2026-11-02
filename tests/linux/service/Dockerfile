FROM debian:trixie-slim
ENV container=docker
RUN apt-get update && apt-get install -y --no-install-recommends systemd systemd-sysv dbus python3 curl ca-certificates && rm -rf /var/lib/apt/lists/*
STOPSIGNAL SIGRTMIN+3
COPY init.sh /hps-init
ENTRYPOINT ["/bin/sh", "/hps-init"]
