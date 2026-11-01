# T15 Linux 透明数据面分批验证

[Issue #20](https://github.com/go-laoji/honk-proxy-server/issues/20) **仍在开发中**。本页保留阶段证据，不把对象加载/挂载或 DNS 成功称为完整业务出口。计划见 [T15 实施计划](../linux-transparent-implementation-plan.md)。

## 捕获契约、报文边界与 socket

已交付独立配置类型、版本化 ABI、无隐式填充的 map 值，以及不分配内存的有界双栈报文元数据解析。处理 IPv4 options、两层可见 VLAN、限定 IPv6 options/atomic fragment；普通分片、路由头、加密扩展及截断不能产生虚构目标。CIDR bypass 可在传输解析前执行。

`hps-net::transparent` 提供透明 TCP/UDP 绑定、完整 UDP 原目标 ancillary 和绑定原源端口的回复 socket。Linux 普通网络测试验证双栈原始目标地址/端口、截断报文拒绝及下一条数据报不受污染；特权测试实际验证非本机双栈绑定、精确 SO_MARK、UDP 回复原源地址/端口。日志 `/tmp/hps-t15-linux-sockets.log`：普通 2 passed / 1 ignored，随后单独执行特权入口 1 passed。配置/ABI/解析器共 12 项通过，Clippy 通过。

配置类型尚未接入 Daemon 启用入口，现有实例不会因新增类型启动捕获。UDP socket 的 SO_MARK 是共享状态：初步实现限制同一 socket 的捕获/绕过类别固定，冲突的混合目标发送明确拒绝并计数；不通过并发修改 mark 冒险走错路。最终产品支持范围随实际多目标验证继续明确。

## 实际内核加载与生命周期

新对象 SHA256：`2e1a3626dce0c2f0597f244fec799329c5b9002601dc426b00d02d41bed22522`。

| 内核与架构 | 实际结果 |
| --- | --- |
| 7.0.14-linuxkit / aarch64 | 7 个新程序通过验证器，5 个 cgroup 与 2 个 TCX link 以禁用策略加载；独占冲突拒绝、程序/map 释放、所有权重新取得通过 |
| QEMU 实际启动 6.12.115 LTS / aarch64 | 同一对象、同一测试通过，`HPS_SELF_TEST_EXIT=0` |
| 更新对象后的 DNS / 两个实际内核 | 双栈 TCP/UDP DNS、对端恢复、PID namespace 排除、程序/map/私有路由回收仍通过 |
| Linux 相关普通回归 / Clippy | platform/net/config/ABI 通过；需特权的入口另行显式执行，不计为普通单测通过 |

原始证据：[7.0 loader](linux-transparent-loader-kernel7-2026-11-02.json)、[6.12 loader](linux-transparent-loader-kernel612-2026-11-02.json)、[7.0 DNS 回归](linux-transparent-dns-kernel7-2026-11-02.json)、[6.12 DNS 回归](linux-transparent-dns-kernel612-2026-11-02.json)。日志 `/tmp/hps-t15-kernel-baseline.log`、`/tmp/hps-t15-kernel612-loader.log`、`/tmp/hps-t15-kernel612-dns.log`。loader 证据显式保存 `forwarding_verified=false`。

实际内核发现并修复：IPv6 context 数组复制和将 ctx 字段地址直接作为 map key 都不满足验证器约束；改为固定偏移读取与栈内 key。Aya 0.14 把 CgroupAttachMode 作为 link_create flags 传入；[6.12 内核 cgroup link](https://github.com/torvalds/linux/blob/v6.12/kernel/bpf/cgroup.c#L1291) 要求该 flags 为零，并自行采用多程序语义，因此使用 Aya 的 Single（零值）而不传 AllowMultiple。6.12 的 TC 程序不能调用 get_netns_cookie；TCX 使用 loader 创建链接时绑定的 namespace 身份，cgroup socket 路径继续用实际 helper 校验。网卡迁移/丢失监控会随下一批生命周期实现补齐。

测试只在私有 Docker netns/cgroup 与 QEMU 内核中创建自有对象，不运行宿主 Mac 网络命令；原 17900 服务和用户应用保持不变。

## 待完成

自有 Rust 策略路由、实际透明 TCP/UDP 交接及上游出口、可信进程规则、内核/用户态原子代际、LAN 范围、接口/路由变化、异常退出恢复、控制面与完整 x86_64/aarch64 矩阵尚未完成。T11/T13 的联合 Linux 门槛和 T15 Issue 保持开放，完整 datapath_ready 仍为 false。
