FROM rust:1.98.1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f
RUN apk add --no-cache qemu-system-aarch64 cpio tar gzip binutils zstd
RUN mkdir -p /qemu/kernel && wget -q -O /qemu/kernel.apk https://dl-cdn.alpinelinux.org/alpine/v3.22/main/aarch64/linux-virt-6.12.115-r0.apk && apk verify /qemu/kernel.apk && tar --ignore-zeros -xzf /qemu/kernel.apk -C /qemu/kernel
COPY run-kernel612.sh /usr/local/bin/hps-test-kernel612
ENTRYPOINT ["sh", "/usr/local/bin/hps-test-kernel612"]
