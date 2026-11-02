# 实际透明数据面矩阵

`capture-matrix.json` 列出每个客体必须实际匹配并通过的 12 个测试。`run-capture-matrix.py` 按 aarch64/x86_64 × 6.12.115/7.2.14 启动真正的 QEMU 内核，保存串口、客体执行文件 SHA256、内核 APK SHA256、逐项退出码和结构化证据。默认不联网；所有数据流、veth、netns、cgroup、供应商和目标属于客体夹具。

## 构建客体内核

镜像中的宿主工具为 arm64，QEMU TCG 可以启动两种客体架构。内核 APK 经 Alpine 随发行版携带的官方公钥验证；跨架构签名用 `/usr/share/apk/keys`，不使用 allow-untrusted。当前固定包的来源是 [6.12.115 virt](https://pkgs.alpinelinux.org/package/v3.22/main/x86_64/linux-virt)、[7.2.14 stable](https://pkgs.alpinelinux.org/package/edge/community/x86_64/linux-stable)。

```sh
docker build --platform linux/arm64 -f tests/linux/ebpf/Dockerfile.matrix -t hps-ebpf-x86-kernel612:dev tests/linux/ebpf
docker build --platform linux/arm64 -f tests/linux/ebpf/Dockerfile.matrix --build-arg HPS_GUEST_ARCH=aarch64 --build-arg HPS_KERNEL_BRANCH=edge --build-arg HPS_KERNEL_REPO=community --build-arg HPS_KERNEL_PACKAGE=linux-stable-7.2.14-r0.apk -t hps-ebpf-arm-kernel72:dev tests/linux/ebpf
docker build --platform linux/arm64 -f tests/linux/ebpf/Dockerfile.matrix --build-arg HPS_GUEST_ARCH=x86_64 --build-arg HPS_KERNEL_BRANCH=edge --build-arg HPS_KERNEL_REPO=community --build-arg HPS_KERNEL_PACKAGE=linux-stable-7.2.14-r0.apk -t hps-ebpf-x86-kernel72:dev tests/linux/ebpf
```

arm64 6.12 镜像使用原有 `Dockerfile`，标签 `hps-ebpf-kernel612:dev`。默认的 `/qemu/kernel/boot/vmlinuz-virt` 由 7.2 客体的 `HPS_GUEST_KERNEL=/qemu/kernel/boot/vmlinuz-stable` 覆盖。脚本解压模块后按 `modules.dep` 只保留 ipv6、af_packet、veth 及实际依赖闭包，防止完整稳定内核的无关驱动占满 initramfs。客体在证书/租约验证前设置真实宿主 epoch，不禁用 TLS 校验。

## 构建实际测试文件

在两个独立平台的 Rust 1.98.1 Alpine 构建容器中分别执行，输出 JSON stdout 并单独保留 stderr：

```sh
cargo test --locked -p hps-platform --lib -p hps-application --test gateway --test transparent --test capture_monitor --no-run --message-format=json
cargo test --locked -p hps-daemon --no-default-features --test linux_capture --no-run --message-format=json
```

分别使用 `/target/aarch64`、`/target/x86_64` 的 `CARGO_TARGET_DIR`、各自 Cargo/Rustup 卷和 `CARGO_BUILD_JOBS=1`。构建只产生测试文件，不能作为通过证据；下一步必须在客体实际执行。

`prepare-matrix.py --messages <JSONL> --output <TSV>` 从 Cargo package/target 选择精确文件，拒绝缺失或歧义。x86 加 `--strip-x86`，使用原生 x86 binutils strip **副本**，避免通用 ELF 输出格式丢失 e_machine。可通过 `--strip-image` 选择已缓存的 x86 镜像。x86 的 busybox 与 `/lib/ld-musl-x86_64.so.1` 从同一 Rust x86 镜像复制到共享卷 `/target/x86-userland/bin/`、`/target/x86-userland/lib/`。

共享卷名为 `hps-validation-target`；eBPF ABI v4 对象路径为 `/target/t15-ebpf/bpfel-unknown-none/release/hps-ebpf`，旧 ABI v3 拒绝夹具为 `/target/t15-abi3-reference-20261102/hps-ebpf`。这些是测试夹具的参数，不是安装包或生产路径。旧对象可由对应历史源码构建得到，不从第三方项目取对象。

```sh
python3 tests/linux/ebpf/run-capture-matrix.py --work /absolute/immutable/source --arm-manifest /absolute/arm.tsv --x86-manifest /absolute/x86.tsv --output /absolute/evidence
```

可选 `--architecture`、`--kernel` 只执行指定组合；子集结果不能替代完整四组合。可选 `--arm-daemon`、`--x86-daemon` 指定冻结的独立 hpsd，避免另一个功能构建替换测试输入。每个组合必须：全部 CASE_EXIT 为 0、每项恰好 `1 passed`、实际 kernel 字段匹配、整体 SELF_TEST_EXIT=0；零匹配、仅编译、其他架构容器或失败启动均不算通过。

源码快照使用新 mtime（例如 `shutil.copy`），不在编译或打包期间覆盖输入。保留镜像架构专用本地别名可以避免经典 Docker image store 在同标签两个架构之间切换而重复拉取；不清理其他项目容器或卷。

## 实际 Linux 控制台

构建最新共享 `dist` 并按顺序构建启用 web-ui 的 hpsd 和 CLI，再用独立容器、cgroup v2 及 [控制台配置](../console/daemon.toml) 启动。默认配置在容器内监听 17996；宿主可映射任意未占用端口，不操作已有服务。

```sh
HPS_LINUX_UI_ENDPOINT=http://127.0.0.1:17997 HPS_LINUX_UI_CONTAINER=hps-t15-console-live npx playwright test --config tests/linux/console/playwright.config.ts
```

用例在同一个真实 Linux owner 上验证 CLI stop/start、浏览器 stop/start、readyz、错误对象路径及持久错误提示、候选保存与显式激活的区别、需要重启的范围变更，以及窄屏。测试容器名称和端口必须属于本次隔离验证；测试完只终止该容器。
