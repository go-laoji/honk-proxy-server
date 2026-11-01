# Rust eBPF 构建与隔离验证

T03 提供本项目自己的 Rust connect4/connect6 程序、共享 ABI、Aya 加载器和可复现的实际内核验证。生产透明 TCP/UDP、持久接管和恢复由 T15 完成。服务的 `compiled/configured/attached/ready` 仍按当前运行实例报告；运行一次隔离自测不会把 Daemon 标为接管就绪。

## 构建

常规 CLI/Daemon 保持仓库稳定 Rust 工具链，不要求桌面或 Rust nightly：

```sh
cargo build --locked -p hps-cli -p hps-daemon --no-default-features
```

eBPF 在独立 `ebpf/` 工作区编译，固定 `nightly-2026-09-01` 和 `rust-src`，用户态 Aya 为 0.14，eBPF Aya 为 0.3。先按 [官方说明](https://github.com/aya-rs/bpf-linker) 安装对应平台的 bpf-linker；本次验证使用 0.12.0。

```sh
rustup toolchain install nightly-2026-09-01 --component rust-src --profile minimal
./scripts/build-ebpf.sh
```

默认产物是 `ebpf/target/bpfel-unknown-none/release/hps-ebpf`，它是内核对象，作为参数交给加载器。构建独立工作区时不要启用 ABI crate 的 userspace feature；常规服务不自动构建或加载该对象。`bpfel` 用于本次验证的小端环境，其他目标需独立验证。

## 本机隔离自测

在目标 Linux 本地，以所需系统能力和可写的 cgroup v2 委派运行：

```sh
hps --output json linux self-test --object ./hps-ebpf
hps --output json linux self-test --object ./hps-ebpf --cgroup-root /sys/fs/cgroup
```

这是本机命令，`--endpoint` 不会让它在远端执行。`hps linux preflight` 与控制台预检读取目标 Daemon 的环境，两者用途在帮助中分别说明。macOS 直接返回 Linux 不支持错误，没有加载动作。

自测在指定 cgroup 根下新建独有目录，只把自己生成的子进程移进去；不附加到根 cgroup，不移动 Daemon 或其他已有进程，不创建持久 pin。程序以 ABI 版本、三个有界 map 和两个 hook 协作：policy array 1 项、per-CPU stats 2 项、socket-cookie LRU 128 项。

它实际执行 IPv4/IPv6 TCP 连接，读取内核计数及 PID/TID/cgroup/cookie/目标上下文，更新 policy 后验证指定端口被内核拒绝，再取消拒绝验证连接成功。随后重新设置拒绝、关闭 hook 所有权，验证连接恢复；查询程序及 map ID 消失，并移除测试目录。map 回收经过 RCU，验证有 3 秒有界等待。成功报告同时包含 `datapath_ready=false` 和 `scope=isolated_connect_hooks`。

对象错误、权限或挂载条件不足会失败退出，不返回成功报告。正常返回与错误路径使用 RAII 回收；强杀进程时内核 FD/link 会关闭，可能留下空测试 cgroup，持久运行的异常恢复由 T15 管理。

## 6.12 LTS 可复现验证

`tests/linux/ebpf/Dockerfile` 固定官方 Rust 镜像 digest 与 Alpine 签名验证的 `linux-virt-6.12.115-r0.apk`。它使用 QEMU TCG 启动实际 aarch64 Linux 6.12.115 内核，运行相同 CLI 和对象，网络仅使用 guest loopback。需要提供 Linux aarch64 musl CLI 与 eBPF 对象；当前验收基线为 aarch64。

```sh
docker build -t hps-ebpf-kernel612 tests/linux/ebpf
# artifacts/ 放入 Linux aarch64 的 hps 和本项目的 hps-ebpf
# 下面的目标只是验证容器中的实际 guest 内核，无宿主网络需求。
docker run --rm --network none \
  --mount "type=bind,source=$PWD/artifacts,target=/artifacts,readonly" \
  hps-ebpf-kernel612 > kernel612.log
python3 tests/linux/ebpf/verify-report.py kernel612.log --kernel-prefix 6.12.
```

`verify-report.py` 只校验 CLI/serial 的实际报告，检查程序/map 回收、IPv4/IPv6 计数、真实拒绝、流量上下文和未声称生产 ready。它不会生成或补齐证据。实际内核日志与当前内核验证的结果见 [验收记录](validation/linux-ebpf-core-2026-11-01.md)。

控制台 Linux 页显示目标内核检查、加载器编译能力、当前服务是否内置对象以及配置/挂载/就绪状态。只读检查没有足够证据时保留 unknown/fail，编译能力作为独立字段报告；没有混入环境通过项。bpffs 条件针对持久 pin 场景，本隔离自测不使用 pin，不能把它当成生产 pin/恢复已验收。
