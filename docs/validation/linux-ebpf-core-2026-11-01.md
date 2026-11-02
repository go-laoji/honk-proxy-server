# T03 自研 eBPF 实际内核验收

日期：2026-11-01，Asia/Shanghai。对应 [Issue #8](https://github.com/go-laoji/honk-proxy-server/issues/8)。自研 Rust 程序、版本化 ABI、Aya loader 与隔离验证完成，未集成 honk 代码、对象、crate 或 fork。

## 实际内核结果

| 环境 | 程序 ID | map ID | IPv4 connect/allow/deny | IPv6 allow | 回收 |
| --- | --- | --- | --- | --- | --- |
| Linux 7.0.14-linuxkit / aarch64 | 104, 105 | 36, 37, 38 | 3 / 2 / 1 | 1 | 程序/map/测试 cgroup 全部验证消失 |
| Linux 6.12.115-0-virt / aarch64 | 5, 6 | 4, 5, 6 | 3 / 2 / 1 | 1 | 程序/map/测试 cgroup 全部验证消失 |

计数来自实际内核 per-CPU map，errors 均为 0。用户态更新 policy 后，测试子进程得到内核权限拒绝；清除 policy 后连接成功。再次设置拒绝并 detach 后，连接仍成功，确认 hook 已解除。socket-cookie map 记录了四个实际子进程的 PID/TID、cgroup、协议、目标和 comm，包含 IPv4 127.0.0.1 与 IPv6 ::1。

7.0 为 Docker Linux VM 的实际内核，在只影响新建 cgroup 的特权测试容器中执行。6.12 使用 QEMU TCG 实际启动 Alpine 签名验证的 `linux-virt-6.12.115-r0.apk` 内核，配置包含 BPF、CGROUP_BPF、IPV6、DEBUG_INFO_BTF；没有网络出口，仅 guest loopback。仓库内的可复现 Dockerfile/启动脚本已重新构建、再次运行，并由报告校验器验证退出标记与 JSON。它不是用 7.0 容器冒充 6.12。

原始报告保存在 [7.0 报告](linux-ebpf-kernel7-2026-11-01.json)、[6.12 报告](linux-ebpf-kernel612-2026-11-01.json)。本次同一 eBPF 对象 SHA256：`691ecabc5217e50f0c3079a270c544b0c17a4da583611f8db26c9e06c462f3ea`。ABI 1；稳定用户态 Rust 1.98.1；eBPF `nightly-2026-09-01` / Rust 1.100.0-nightly / LLVM 23.1.0；bpf-linker 0.12.0。官方 aarch64 musl linker 发布文件 SHA256 已校验为 `5e201e69dafac7af98f9788f1c68edd5ccc8fa0805093f899bf02bd2947b3935`。

## 失败路径与普通构建

实际无特权容器因 cgroup 只读失败，退出码 5，不返回成功 JSON。实际损坏 ELF 在 loader 报解析错误，退出码 5，新建测试目录已清理。macOS CLI 在没有有效对象、没有 Daemon 的情况下明确报告需要 Linux，不触碰 cgroup。

实际内核发现并修复了 PID 格式化写可能拆成两次命令的问题：cgroup.procs 采用一次完整 PID 写入。程序/map 释放后有 RCU 回收延迟，验证采用最多 3 秒有界等待并只接受 ENOENT，不把其他权限错误当成已移除。正常路径和错误路径使用自有 RAII 生命周期，不创建 pin。

普通 Rust 回归：macOS **84 项**、Linux aarch64 musl **83 项**通过（差异为非 Linux 明确不支持的专用用例）；两端 clippy 通过。新增 ABI 布局测试和非 Linux 结果检查。既有 T07 空闲测试原先把两秒租约与两秒空闲窗口重叠，即时 timer 首次执行时合法补池会使断言抖动；测试改为租约明确晚于空闲窗口到期，业务策略未更改，聚焦及完整回归均通过。

浏览器完整 **11 项 E2E**通过，最终预检编译字段分离后重跑 Linux 页面通过。页面从实际 Daemon 读取条件、内核和加载器能力，刷新不加载程序。缺失环境证据的既有测试保持原断言，编译能力作为独立字段报告。fmt、TypeScript、Prettier、构建及 Desktop .app 打包通过。

## 交付边界

使用方式见 [构建和自测](../linux-ebpf-usage.md)。上述真实程序与 map 生命周期完成 T03 的前置验证；报告 `scope=isolated_connect_hooks`、`datapath_ready=false`，没有宣称已提供完整透明代理。生产透明 TCP/UDP、持久挂载/恢复仍在 T15；DNS 接管由 T09 联合实现。只读预检、内置对象、已挂载和接管就绪在控制台分别显示。验证基线是实际 6.12.115 与 7.0.14 的 aarch64 环境，未验证的内核/架构不作等同承诺。
