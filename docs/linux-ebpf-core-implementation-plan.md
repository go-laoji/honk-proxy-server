# T03 自研 eBPF 程序与加载验证计划

继续 [Issue #8](https://github.com/go-laoji/honk-proxy-server/issues/8)。本地分支 `feature/linux-ebpf-core` 由已验收的健康分支开启；不集成 honk，不自动推送、合并或结束 Git Flow feature。

## 交付切片

1. 独立 `no_std` Rust eBPF 工作区与共享版本化 `repr(C)` ABI。自行编写 cgroup connect4/connect6 程序，使用有界 policy、per-CPU 统计与 socket-cookie 观测 map。基础观测允许连接；隔离自测可切换明确测试端口的拒绝以证明用户态写 map 实际影响内核执行。
2. Linux 专用 Aya loader，检查对象、ABI 和 map 规格，RAII 保有程序/挂载。普通 macOS 构建和无桌面 CLI 不要求 eBPF/nightly。eBPF 对象单独固定工具链构建，编译器、linker、对象 SHA、内核与挂载证据写入验收记录。
3. 本地 `hps linux self-test` 使用新建的独立 cgroup，仅把自己的短生命周期子进程放进去；不挂根 cgroup，不移动 Daemon 或用户进程。验证 IPv4/IPv6 真实连接、内核计数/上下文、用户态更新 map 后真实拒绝，再 detach、验证连接恢复、确认程序/map ID 不再存在并清理自己创建的 cgroup。无特权或错误环境返回具体失败，不假报成功。
4. CLI/预检/控制台区分只读条件、编译能力、隔离验证与生产接管。T03 的探针及自测不是完整透明 TCP/UDP 出口，持久运行、恢复和真实转发仍由 T15 交付；DNS 接管在 T09 联合验证。

## 环境与验证口径

当前可用的 Docker Linux 为 aarch64 `7.0.14-linuxkit`，BTF 可读，特权隔离容器中 cgroup v2 可创建并清理子目录，已作实际验证。该检查尚未加载程序。初始目标还包括 6.12 LTS；其实际验证需单独的真实 6.12 内核环境，不能以编译兼容或当前内核结果代替。对任何未实际运行的内核如实标记未验证。

参考 [Aya 开发环境](https://aya-rs.dev/book/start/development/)、[Aya 0.14 CgroupSockAddr](https://docs.rs/aya/latest/aya/programs/cgroup_sock_addr/struct.CgroupSockAddr.html)、[官方 bpf-linker 构建方式](https://github.com/aya-rs/bpf-linker)。使用 Rust nightly 与独立目标构建 `core`；标准服务仍固定仓库稳定工具链。验证时只为本项目测试进程使用隔离容器的权限，不改变 macOS 系统代理或其他服务。
