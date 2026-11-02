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

## 共享 TCP 引擎入口（第四批，普通引擎契约）

`Engine::with_capture` 仅在 Linux 提供，明确准备进程规则能力；显式入口构造仍拒绝未提供的进程能力。`spawn_captured_tcp` 检查原始 socket 两端地址、正代际与引擎代际、TCP 和来源；本机缺少可信进程拒绝，Gateway 的 process 永远清空。接入同一 Dialer/router/group/health/lease/node/observation，复用全局连接额度与带宽；独立任务登记与 shutdown 共用互斥，停止等待全部捕获任务。重复设置代际不再改变已创建引擎的拨号规则版本。

普通 Linux `captured_tcp_contract_shares_process_rules_group_admission_bytes_disconnect_and_immutable_revision` 通过 HTTP、HTTPS、SOCKS5 三类真实上游路径验证：进程规则选定固定组/节点、业务字节、源端口、旧代际拒绝、端点不符拒绝、缺少本机进程拒绝、全局额度耗尽、主动断开回收、Gateway 不匹配进程规则。该项进程输入是测试构造的可信调用契约，不是内核 PID 证据；内核到共享引擎的联合验收仍待下一批。Linux/macOS 全部 engine tests 与 Clippy 均通过。

## 实际内核到共享引擎 TCP（第五批）

`transparent::tcp_tests::real_transparent_tcp_shared_engine` 在实际 7.0.14-linuxkit 和 QEMU 6.12.115-0-virt 均显式通过，证据为 `linux-transparent-tcp-shared-kernel{7,612}-2026-11-02.json`，对象 SHA256 与第三批相同。受控客户端在指定 cgroup 发起双栈原目标 TCP；TCX 接到透明 socket 后，`Capture::routing_context` 仅根据实际两端地址和命名空间查询自有流 map，验证 ABI、正代际、来源、cookie/PID/TID/cgroup 与 verified 标记后构造引擎上下文。本机元数据缺失拒绝，网关不返回进程。

6 条真实 TCP 经过实际 Linux comm 进程规则、三个固定组，分别走 HTTP CONNECT、HTTPS 上游 CONNECT、SOCKS5 上游，再经上游引擎 Direct 到实际受控目标；TLS 上游正常校验证书。实际客户端 PID 与连接登记相符，原目标不变，规则代际 7，业务上行合计 102 B、下行 114 B；上游连接登记为 Explicit、无进程、Direct。测试地址通过 Rust route-netlink CREATE|EXCL 添加到自有隔离 netns，退出仅删除本次 ACK 所属地址；没有在后端调用 ip/iptables。测试也核对捕获 map 无错误及丢失进程计数。

这一项的 `forwarding_verified=true` 仅表示已验收的本机 TCP 共享引擎路径。UDP 共享引擎、LAN、完整代际发布/回滚、Daemon 控制面和异常恢复尚未完成，因此 `full_transparent_datapath_ready=false`，T11/T13/T15 不关闭。第四批引擎回归最终统计 Linux 39 / macOS 38 通过，均无失败或忽略；第五批平台相关两端 Clippy 通过。

## SOCKS5 UDP 传输基础（第六批）

依据 [RFC 1928](https://www.rfc-editor.org/rfc/rfc1928.html) 增加独立 UDP ASSOCIATE 传输；TCP CONNECT 与 UDP ASSOCIATE 共享上游供应商协商/认证实现。控制 TCP 结束或异常后终止 association，发送/接收均同时观察控制连接；没有独立遗留监视任务。外部/NAT 端点未知时请求全零地址，接受合法 wildcard 回复并替换为实际 TCP 对端；域名 relay 通过 Bootstrap resolver 解析，当前要求 relay 与 TCP 对端同地址族。UDP descriptor connect 到协商出的 relay 地址和端口，内核丢弃其他来源。

帧编解码支持 IPv4、IPv6、域名和空业务报文；非零 FRAG、非法地址/端口/保留位、异常长度拒绝。SOCKS 完整帧保守上限 65507 B，业务上限扣除真实地址头；接收缓冲为 65536 B，防止内核截断大报文后误认为有效短报文。调用方缓冲不足则消费并拒绝整个报文，既不返回部分业务内容，也不重放。失败认证、拒绝 UDP 命令和超时都不产生可用 association。

Linux/macOS 真实本地 socket 测试覆盖双栈匿名/供应商认证、wildcard/domain relay、实际 UDP 目标收发、陌生 relay 来源丢弃、FRAG 消费拒绝后仍可收正常报文、尺寸边界、控制 TCP 关闭、超时与拒绝。完整 engine 回归 Linux 43 / macOS 42 通过，零失败/忽略，两端 Clippy 通过。本批仅为出站传输基础，尚未接入共享 UDP 规则/池/会话 owner；控制台 UDP 数据面能力仍保持不可用，不据此关闭 T15 或 T19。

## 共享 UDP 出站策略与事实（第七批）

`Dialer::for_datagrams` / `open_udp_routed` 接入相同首匹配规则、固定/显式 fallback 组、动态池、租约/健康与节点操作门槛；TCP 与 UDP 共用同一 admission、endpoint dial、失败分类与反馈方法。UDP 只选择 SOCKS5 潜在出口，HTTP/HTTPS 标记 tcp_only；UDP 补池信号只发送给明确声明 SOCKS5 的供应商，不能激活 HTTP 或未知协议供应商的采购。Direct/Reject 不产生采购信号。UDP 支持明确 Direct，报文不重传；Socks 控制连接、DemandGuard 与 NodeGuard 生命周期由实际 DatagramConnection 持有。

UDP 连接观察使用原有全局事实源；业务字节只在发送成功和有效来源回复后增加，SOCKS 头/握手、错误地址和无效数据报不计入。主动断开立即唤醒 UDP I/O 并回收节点占用；drain 阻止新 association，已有 association 可继续收发。当前健康门槛沿用既有 TCP 健康事实，**并未增加 UDP 健康探测或把 TCP Healthy 宣称为 UDP Healthy**；association 可用性还必须经过真实 UDP ASSOCIATE，实际目标收发由业务结果验证。

普通双栈测试使用实际 SOCKS5 UDP relay 和 UDP echo：健康未知拒绝、有效健康后固定组显式 fallback、错误回复来源拒绝且不计数、真实字节、drain 保留已有会话、主动断开、到期节点不连接、Direct/Reject 不补池、UDP 仅产生 SOCKS5 供应商需求信号。完整 engine 回归 Linux 46 / macOS 45 通过，零失败/忽略，两端 Clippy 通过。还没有 Linux UDP owner/全局排队预算/代际和透明回复的联合证据，控制台 UDP 数据面能力保持不可用。
