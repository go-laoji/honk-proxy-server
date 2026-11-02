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

## 私有策略路由（第二批）

Rust 直接通过 route-netlink 建立私有表 10530 的双栈 local route 与精确 fwmark rule（priority 31530、protocol 200），无需外部 ip 命令。每次交换有序列/内核发送者检查、3 秒期限和 4 MiB 响应上限。先检查外部表/规则/mark 冲突，再写入私有、限长、原子落盘的命名空间归属日志；CREATE|EXCL 成功收到 ACK 的对象才进入本次回滚集合。启动可从日志恢复前次遗留，退出仅清理精确归属对象。

实际 7.0.14-linuxkit 与 QEMU 6.12.115-0-virt 均通过：保留外部 protocol 99 路由、保留同优先级外部规则、双栈创建、丢失用户态状态后的日志恢复、精确清理及日志归零。对应 `linux-transparent-routes-kernel7-2026-11-02.json` 和 `linux-transparent-routes-kernel612-2026-11-02.json`。Linux Clippy 与 macOS check/format/diff 检查通过。该测试没有 SIGKILL，也没有业务转发，`forwarding_verified=false`。后续完整 Daemon 异常恢复与真实 TCP/UDP 仍为独立验收门槛。

## 双栈原目标、进程与 UDP 首包（第三批）

新对象 SHA256 `466a946dd961ca1aff226bfb2e37484bf7fd7cce4f357b7acb55a7cce16e985d`，新增 sock_create，共 8 个透明捕获程序。实际 aarch64 7.0.14-linuxkit、QEMU aarch64 6.12.115-0-virt 均通过 12 次原目标/进程关联：TCP 双栈、UDP 连接式/无连接式各双栈及同 socket 多目标；回复源地址与完整端口保持原目标，元数据来自实际 socket cookie/可信 PID/cgroup，代际均为 7。证据为 `linux-transparent-packets-kernel{7,612}-2026-11-02.json`。这是捕获 socket 到受控回复的验收，还没有接入上游与共享业务引擎，因此 `forwarding_verified=false`。

首包测试暴露并修复两项内核事实：

- local 默认路由未指定首选源时，IPv4 会选远端目标自身为源，TCP 在发包前拒绝；现在指定 127.0.0.1/::1 作为本机捕获的默认源。明确绑定的本机源与 LAN 报文源不改写。本机规则看到的是捕获路由实际选出的源，不能把它解释为原公网出口地址。
- [Linux 6.12 UDP sendmsg](https://github.com/torvalds/linux/blob/v6.12/net/ipv4/udp.c) 在 cgroup 发送钩子之前已经快照路由 mark。[IPv6 同样如此](https://github.com/torvalds/linux/blob/v6.12/net/ipv6/udp.c)。新增 sock_create 初始化受选 cgroup 的新 UDP socket，Daemon PID、其他 namespace、外部 mark 继续旁路。启用前已存在于选中 cgroup 的无连接 UDP socket，首个捕获发送明确返回 EPERM，初始化后由调用方决定是否再次发送；服务不代替调用方重发数据。两族均实际测试首次拒绝、随后捕获。进入 cgroup 前创建的 socket 属于原 socket cgroup，不在该捕获范围；应用需在目标 cgroup 内重建 socket。

CIDR/UDP 端口旁路使用独占 mark 的精确 GOTO 规则与自有 NOP 恢复目标，恢复到捕获规则之后的原规则链，不将 main 无路由错误回落到 local 捕获表。保护 priority-1、priority、priority+1 三个优先级；有旁路条件时配置范围为 2..32764。双栈 CIDR/端口、0/0 全旁路、正常不可达、归属日志恢复与精确清理均在两个内核通过，证据为 `linux-transparent-udp-routes-*`。归属日志升级为严格格式 2、上限 64 KiB；仅接纳已验证的内核规范化字段，如 7.0 导出的全 1 端口掩码，以及 IPv6 查询中值完全相同的重复首选源。冲突值或未知条件仍拒绝，修改后的归属标记保留日志而不宽泛删除。

相同对象的禁用加载、资源回收与原 DNS 捕获回归在两个内核通过（`linux-transparent-udp-loader-*` / `linux-transparent-udp-dns-*`）。普通 Linux 库测试 16 通过、1 个特权项忽略；特权项已独立显式通过。Linux/macOS 相关 Clippy、格式检查通过。TCP/UDP 上游转发、原子代际发布、LAN、状态监视与完整 Daemon 恢复仍未验收，T11/T13/T15 保持开放。
