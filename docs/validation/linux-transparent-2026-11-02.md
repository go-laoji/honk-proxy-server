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

## 透明 owner 的额度、排队与停止（第八批）

加入共享 CaptureBudget 与 scoped TCP/UDP owner：TCP、UDP setup 共用 pending permit；UDP owner 另持会话 permit。排队报文与字节在分配 payload 前获取 permit，并一直持有到发送或丢弃，空数据报也占报文额度；队列、在途和失败清理均回收相同 permit。引擎代际间继续共享原全局连接 gate，不能通过新建代际或预算绕过旧流占用。UDP owner 的 sender、reply 与独立 SOCKS 控制监视均在一个受跟踪任务内，限速等待期间控制连接仍被监视，不产生遗留子任务。

回复最终 try_send 与 session.cancel 共用短互斥；取消返回后旧 owner 不能继续给已重用客户端元组发回复。空闲回收、scope 取消、全局退出均释放任务/socket/队列/节点额度；捕获 scope 的停止不停止显式 HTTP/HTTPS/SOCKS5 入口。drain 的 TaskTracker 正确 close/wait/reopen，取消 drain 等待也恢复接纳，避免一次超时后永久拒绝新的捕获流。

Linux 普通真实 socket 测试验证：pending=1、会话=1、报文=2、字节=512 的上限，512 B 与空报文排队/在途占用、字节/报文/尺寸分别拒绝、setup 完成释放 pending、旧代际占用阻止新代际、取消阻断延迟回复、显式入口保持监听、空闲结束回收、scope 取消 pending 关联与排队。还分别验证 TCP 控制关闭能打断上行 512 秒限速等待，以及已收到下行数据后的 50 秒限速等待（发送侧无下一条消息时也必须监视）；两项均在 1 秒内回收。TCP 三类上游契约同时验证 scoped pending 在业务连接仍活跃时已释放，等待 drain 被取消后接纳恢复。该批 owner 进程元数据仍为测试调用契约，实际内核 UDP 联合验证待下一批。

额度中的 queued_bytes 指客户端 UDP 请求的等待/在途 payload，不是进程或内核 socket 总内存上限；每会话还有有界接收缓冲/socket 开销，受 max_udp_sessions 约束。当前公共实际服务尚未接入 owner，不据此宣称完整数据面 ready。Linux 全部 engine 用例 50 通过，macOS 45 通过，零失败/忽略，相关 Clippy 通过；实际 7.0 内核的共享 TCP 三上游回归也通过。

## 实际内核到共享引擎 UDP（第九批）

`transparent::udp_tests::real_transparent_udp_shared_engine` 在实际 aarch64 7.0.14-linuxkit 和 QEMU aarch64 6.12.115-0-virt 均显式执行 1 项并通过，证据为 `linux-transparent-udp-shared-kernel{7,612}-2026-11-02.json`。同一对象 SHA256 `466a946dd961ca1aff226bfb2e37484bf7fd7cce4f357b7acb55a7cce16e985d`，每个内核分别验证 Direct 与 SOCKS5 两种模式，各 8 条真实流：双栈、连接式/无连接式 UDP、同一 socket 两个目标。进程规则来自实际 map 中客户端 PID/comm，代际为 7；SOCKS5 模式经过固定组、实际 UDP ASSOCIATE 和受控 UDP relay 到真实目标。每种模式业务上行 144 B、下行 128 B，回包原始源地址与端口均核对。

Daemon 测试进程及其工作线程在启用前进入受选 cgroup；其新建 Direct 与 relay 出站 socket 仍正确旁路，不发生自身递归捕获。每种模式保留全部客户端 socket 到结束，避免测试中的端口提前复用；8 个 owner 在 1 秒空闲配置下全部回收，排队/会话/pending 额度归零。停止后核对实际 TCX 链接与私有路由回收，并恢复测试进程原 cgroup。双内核均没有 metadata 缺失、捕获错误、无效回复或队列丢弃。

共享地址/cgroup fixture 提取到独立测试模块，使用 Rust route-netlink CREATE|EXCL 并按 ACK 所有权清理。Linux 与 macOS 相关 Clippy、格式检查通过。一次旧快照执行只匹配 0 项，不计入验收；本批证据均来自刷新源码后实际匹配的 1 项测试与两个模式的结果。

这些证据覆盖本机 UDP 共享引擎路径，`forwarding_verified=true`；完整 Daemon 控制面、LAN、原子代际、接口/路由监视、异常恢复和 x86_64 矩阵仍未完成，`full_transparent_datapath_ready=false`，T11/T13/T15 继续保持开放。

## 完整内核策略快照发布（第十批）

ABI 升级为 v2：策略从原地更新的 Array 改成单元素、NO_PREALLOC 的 HashMap。加载器在挂载前严格核对实际 map 类型、flags 与容量，旧 Array 对象拒绝加载。依据 [Linux 6.12 hashtab 实现](https://github.com/torvalds/linux/blob/v6.12/kernel/bpf/hashtab.c)，普通 hash 更新先分配并写完新元素，再通过 RCU 加入替换；本项目不原地修改策略元素。启用/热更/禁用均发布完整策略值。

新对象 SHA256 `d46c290a48354f6751b8b85e493441e693c007414442809239986b06cf8cee7e` 在实际 aarch64 Linux 7.0.14 与 QEMU 6.12.115 均通过：10,000 次完整 56 字节策略切换，两个独立 FD 的并发读取线程各至少 10,000 次查询，只观察到完整旧值或新值，混合快照为零。该压力项验证实际内核 map，不挂载链接或路由，不把它解释为完整业务热更。证据与新对象 TCP/UDP 回归一起保存于 `linux-transparent-policy-kernel{7,612}-2026-11-02.json`。

相同新对象在两个内核重新通过六条 TCP 三类上游、Direct/SOCKS5 各八条 UDP 的实际业务路径，相关 Linux/macOS Clippy 和格式检查通过。当前只完成内核策略发布基础；流元数据刷新、用户态代际保留、配置持久 CAS 与回滚仍待联合验证，完整 ready 保持 false，T11/T13/T15 不关闭。

## 不可变流身份与精确回收（第十一批）

ABI v3 要求进程和流使用 NO_PREALLOC 普通 HashMap，加载器拒绝旧 LRU 对象。内核只用 NOEXIST 创建身份；进程元数据固定为该 socket 首次受选发送的 PID/TID/comm/cgroup 快照，同 PID 后续线程或 comm 变化不覆盖它，跨 PID 使用已有身份拒绝。flow 的身份/代际/初始序号在退休前保持不变；last_seen 只更新原 map 节点的一个对齐时间戳，不把旧完整副本重新写入 map。活动时间是近似 GC 提示，不作为身份或原子发布依据。

元组已有不同 socket cookie、来源或 TCP 初始序号时明确拒绝，不借用旧可信进程或改写代际；LAN 与本机来源冲突同样拒绝。达到 max_flows 后保留已有流，丢弃新流并计数，不通过 LRU 驱逐活跃元数据。用户态提供有界扫描和按完整身份精确退休的方法；内核不替换已有身份、所有用户态删除共用同一互斥，因此旧 owner 不能误删后来的新身份。调用方必须先取消并等待相应业务 owner，生命周期和周期 GC 将在完整运行时接入。`routing_metadata` 从同一次实际 map 查询返回路由、key 与身份，避免两次查询拼出不同元组生命期的数据。

实际 aarch64 7.0.14 与 QEMU 6.12.115 都通过 73 条数据报的内核身份验收：双栈旧流在策略 7→8 后保持 7；精确退休后同 socket 的新 association 进入 8；旧代际退休请求拒绝且新流仍保留；comm 改名后仍是首次受选发送快照；两个独立 cookie 争用被丢弃；64 条元数据满容量时第 65 条丢弃，全部原身份仍在；回收一条后由客户端主动重试才接纳新流。69 次精确退休后 map 为空，路由/链接正常回收。对应 `linux-transparent-identity-kernel{7,612}-2026-11-02.json`。

新对象 SHA256 `ae2c694f313abbc61c7c7a36dee119bb06dadaff3336d6ccf73ddac0a59143e3` 同时重新通过两个内核的完整策略快照压力项、TCP 三类上游与 Direct/SOCKS5 UDP 回归；ABI 普通 5 项、Linux/macOS 相关 Clippy、格式与 diff 检查通过。本批为内核身份/代际基础，不代表持久配置和完整 Daemon 热更已经联合通过。完整 ready 仍为 false，T11/T13/T15 保持开放。

## 每条业务 owner 的结束通知（第十二批）

TCP scoped 入口返回独立 CapturedTcpSession；UDP owner 也增加 finished/is_finished。取消请求与结束事实分开：结束通知的 guard 由实际受跟踪任务持有，在内层流任务返回、socket/节点占用/全局额度/pending/session/队列资源全部释放后才触发；任务未轮询即被丢弃也不会遗留一个永远等待的通知。丢弃句柄不隐式取消业务，原 TCP 便捷入口仍保持原调用语义。

共享 TCP 的 HTTP/HTTPS/SOCKS5 用例验证活动期间未结束，管理断开后仅等待该流通知即可看到额度释放。UDP 用例仅取消并等待自身 owner，无需关闭整个捕获 tracker 或普通显式入口；延迟回包仍被阻断，空闲回收也产生完成通知。运行时可据此先等待对应流结束，再按第十一批的精确身份退休，避免为单流 GC 暂停所有 TCP/UDP 接纳。

完整 engine 回归 Linux 50、macOS 45 项通过，零失败/忽略，相关 Clippy 通过。实际 7.0 内核再次执行 TCP/UDP 两项，六条 TCP 三类上游及 Direct/SOCKS5 各八条 UDP 全部通过，证据为 `linux-transparent-completion-kernel7-2026-11-02.json`。完整 Daemon、控制面、LAN 与最终矩阵仍待接入，full ready 保持 false。

## 实际挂载、命名空间与退休屏障（第十三批）

新增有界 BPF_PROG_QUERY，按实际 cgroup FD 和六个 attach type 核对程序 ID；同时核对 cgroup 路径/FD 的 dev+inode、网络命名空间 cookie、网卡原 ifindex 和 TCX 实际查询结果。启用前必须通过这些检查。不能以仍持有的程序/map ID 代替实际挂载；路由准备/检查/清理遇到错误 namespace 均拒绝操作，并保留原归属日志。

流 map 的独立 FD 与短互斥从配置/路由 State 分开，元数据 lookup、扫描、精确退休不会排队等待可能耗时的 route-netlink 检查。停止时同时释放该独立 FD，避免 map 被无意保留。

`settled_revision` 在配置状态互斥下关闭本次独占的 protocol=0 AF_PACKET socket，利用本项目验证内核 [6.12 packet_release](https://github.com/torvalds/linux/blob/v6.12/net/packet/af_packet.c) 和 [7.0 packet_release](https://github.com/torvalds/linux/blob/v7.0/net/packet/af_packet.c) 中无条件的 synchronize_net，建立策略发布后的网络 RCU grace period。protocol=0 不注册收包钩子、不接收/检查报文、不创建网卡或路由。该退休基础要求 CONFIG_PACKET 和 CAP_NET_RAW；失败时必须保留旧代际，不能用固定延时冒充完成。完整运行时仍须把发布、屏障、元数据扫描和快照回收串行化。没有采用 membarrier GLOBAL，因为验证内核在单个 online CPU 时可能跳过 synchronize_rcu。

实际 aarch64 7.0.14 与 QEMU 6.12.115 均通过：分别卸载 sendmsg4 与 sock_create，同时保留独立程序 FD，仍能发现挂载已丢失并拒绝下一次启用；新线程进入另一 netns 后，挂载/路由操作拒绝，原日志逐字不变且原路由仍存在；持有路由 State 时元数据查询仍可完成；退休屏障返回实际代际 7；停止并关闭独立测试 FD 后全部 map 回收。两个实际内核的证据为 `linux-transparent-inspection-kernel{7,612}-2026-11-02.json`。7.0 的 TCP/UDP 转发回归两项也通过，Linux Clippy/格式/diff 通过。完整 Daemon 和最终矩阵尚待完成，full ready 保持 false。

## Daemon 生命周期与持久代际接入（第十四批）

默认关闭的 `DaemonConfig.linux_capture` 接入配置校验、相对对象路径解析和 Supervisor；非 Linux 在准备时明确拒绝启用。捕获身份、对象内容、预算或绑定变化要求重启。Linux 启用后使用相同 Engine::with_capture、拨号/选择/健康/租约/连接观察，启动时在载入健康与供应商事实后才发布内核策略。与 DNS 捕获共存时补齐双栈自有 alias 旁路，已有覆盖前缀不重复添加。

运行对象持有有限 listener、共享预算、每条 TCP/UDP owner 与不超过 32 个引擎代际。循环使用 Weak，停止后先结束接纳/接收/GC，再逐条等待业务资源释放并回收内核对象。每次热更在持久 CAS 后登记候选引擎、RCU 发布完整内核策略；捕获流只按实际 map 中的 revision 找引擎，缺失时拒绝。仅改实例名称也创建对应代际，不能让内核新 revision 落到旧 generation 的 Engine。

发布与 GC 共用串行互斥：发布后的退休屏障、流 map 扫描、owner 精确退休与旧引擎回收纳入同一序列。内核 pending 身份有 30 秒上限；已结束 TCP 为 FIN/ACK 留 2 秒安静期；UDP 仅在 owner 结束后退休，已收到的旧身份报文不改代际重放。待接纳的旧代际也固定数据库版本，并计入 draining_generations 与代际预算，避免 workspace/prune 只按用户态 active 数回收。

实际 aarch64 7.0.14 与 QEMU 6.12.115 的 Supervisor 联合项均通过：持久 SQLite 启动，真实 cgroup PID/comm 命中 Direct 规则，双栈各一条 TCP/UDP；revision 2 改为 Reject，revision 3 仅改实例名称，4 条旧连接继续按 revision 1 收发，8 条新连接分别按 revision 2/3 拒绝。业务上下行各 24 B；管理断开、安静期 GC、旧代际排空、8 个自有程序回收、从持久 revision 3 重启均通过。证据为 `linux-transparent-daemon-kernel{7,612}-2026-11-02.json`。

Linux config/application 普通回归 46 passed / 4 个特权入口 ignored（该批实际联合项另行显式通过）；macOS 45 项普通回归及新增非 Linux 拒绝项 1 项通过，相关 Clippy、格式和 diff 检查通过。首次联合项仅因测试误用 connection state=active 而未断开 open UDP 导致等待超时，修正后重验，未把该失败计为通过。

本批不代表完整 T15：失败注入/自动恢复、实际 LAN、控制面 API/CLI/MPA 与 x86_64 联合矩阵仍待完成，公开完整 datapath_ready 继续保持 false，T11/T13/T15 不关闭。

## 实际策略 map 发布失败与持久回退（第十五批）

增加 SQLite 单事务的 rollback_publication：将失败激活从 prepared 改为 failed，并恢复之前的 desired/last_applied；若原 desired 尚待重启，继续保持 pending_restart，不把它覆盖为当前运行版本。监听、DNS 捕获和业务透明发布的启动后失败均走该补偿路径。共享 DNS 捕获实例在失败时恢复旧 resolver；新建捕获才回收，避免关闭旧实例。

实际 aarch64 7.0.14 与 QEMU 6.12.115 均通过 BPF_MAP_FREEZE 注入：运行 revision 1 的 4 条真实双栈 TCP/UDP 流，先将 revision 2 设为待重启，再冻结实际已挂载程序使用的策略 map，尝试发布 Reject revision 3。内核拒绝更新后 desired=2、running=1，revision 3 激活准确记录 failed，没有遗留 prepared；4 条旧流仍按 revision 1 收发，业务上下行各 16 B。退出明确返回策略 map 禁用失败，同时仍 detach/回收自有程序和路由；随后能从持久 desired revision 2 重新取得归属并启动。

同一最终源码的正常三代际路径也在两个内核复验通过，证据为 `linux-transparent-rollback-kernel{7,612}-2026-11-02.json`。Mac storage/application 普通回归 65 项、Linux 相关库 26 项通过，相关 Clippy 和格式/diff 通过。QEMU initrd 在 depmod 前展开模块压缩，使最小 BusyBox 加载 CONFIG_PACKET 时不再先提交压缩的 ELF；复验没有 Invalid ELF header 输出。

本批的失败注入针对真实内核更新，未用 mock 替代。DNS 与业务同时启用时的 resolver 回退仍待下一批联合验证；LAN、异常退出恢复、控制面和 x86_64 矩阵未完成，完整 ready 保持 false。

## DNS 与业务捕获同时启用及 resolver 回退（第十六批）

在相同真实 cgroup 内同时启动 8 个 DNS 与 8 个业务透明程序。实际 aarch64 Linux 7.0.14 和 QEMU 6.12.115 均通过：4 条原有业务 TCP/UDP 双栈流继续转发，客户端另以双栈 TCP/UDP 查询 8 次 DNS，收到原 nameserver IP/端口的回复。Daemon 本身也进入受选 cgroup，实际向受控 UDP resolver 出站正常旁路。

保留 revision 2 待重启后，冻结业务策略 map，候选 revision 3 同时改为 Reject 并切换到会返回不同地址的 DNS upstream。业务发布失败回退后，原 4 条流继续工作；随后所有 DNS 查询仍到旧 resolver、返回旧地址，候选 upstream 的实际请求计数为零。查询名区分阶段、协议和地址族，不能由旧缓存遮蔽错误的 resolver 切换。desired/running、failed 记录、停止回收 16 个自有程序和持久重启均核对通过。

证据为 `linux-transparent-dns-coexist-kernel{7,612}-2026-11-02.json`，相关 Linux Clippy、格式/diff 通过。本批补齐第十五批共享 DNS resolver 回退的真实联合证据。LAN、运行监视与恢复、控制台 API/CLI/MPA、异常退出和 x86_64 矩阵仍待完成，完整 ready 保持 false。

## 故障暂停与对象 ABI 边界（第十七批）

ABI v4 加入显式 suspended 策略：保留已配置范围与 bypass，对受选 TCP/UDP 新发送明确拒绝；TCX 对已选中的暂停流丢弃，恢复 activate 后清除暂停位。该故障状态与用户主动停止捕获区分，供运行监视使用。TC egress 在读取进程/流元数据前再次执行地址/端口 bypass，保持其他钩子改写目标后的旁路优先级。published_policy 从实际 map 查询并核对完整已发布快照，退休屏障也核对实际状态，不能仅使用缓存的 revision。

对象包含 `HPS_TP_ABI4`：只允许程序读取，加载器初始化后使用 BPF_MAP_FREEZE 冻结。加载前严格核对该 marker 的类型、容量、flags；此前 ABI v3 即使其它 map 布局相同，也会在任何程序挂载前拒绝，避免旧程序把未知 ABI 当作禁用后直出。新对象 SHA256 `fa5449609f86a6b50300a83a6e10d2f2150aa42eaa6676c8e64dd1581bbd5286`。

实际 aarch64 7.0.14 和 QEMU 6.12.115 均通过：双栈 TCP 各一次暂停 connect 返回 EPERM，恢复后六条 TCP 分别经过 HTTP/HTTPS/SOCKS5 出口；双栈 UDP 各一次暂停 send 返回 EPERM，恢复后由客户端重试，原流代际保持，73 条原身份/容量用例仍通过。ABI marker 的再次写入失败，保留的真实 ABI v3 对象在挂载前拒绝。cgroup 失联、跨 namespace 拒绝、元数据锁独立和 map 回收均重新通过。

相同新对象的 Daemon 正常三代际，以及 DNS+业务发布失败回退两项也在两个内核复验通过。证据为 `linux-transparent-suspend-kernel{7,612}-2026-11-02.json`。相关 Linux/macOS Clippy、ABI 普通测试与格式/diff 通过。运行监视/自动恢复及其控制面、LAN、SIGKILL 和 x86_64 矩阵仍待完成，完整 ready 保持 false。

## 实际服务状态与故障监视（第十八批）

增加共用 LinuxCaptureStatus/Overview DTO：实际平台/范围、对象指纹、运行与内核代际、暂停位、listener 地址、link/route 检查、program/map ID、在途代际、flow/owner 数、额度与内核三阶段计数。诊断并发有界，发布/停止进行中报告 busy，不能把未检查的结果当作已挂载或已消失。管理请求预留 expected_revision 和 expected_epoch 的一致性边界，不增加项目账号或权限。

运行 GC tick 同时检查真实 cgroup/TCX、路由与实际内核策略。故障在发布互斥下先设置 suspended，再取消自身接纳/业务 scope；现有显式入口不受该 scope 取消影响。此批只完成监视与暂停，尚未开启自动重试，状态中 max_retry_attempts=0。

实际 aarch64 7.0.14 和 QEMU 6.12.115 均通过新监视项：启动检查真实 ready、四个 listener、八个程序和匹配代际；精确删除自有 IPv4 capture rule 后，报告 routes_present=false、links_present=true、suspended=true、ready=false。随后实际 cgroup 子进程的 IPv4/IPv6 TCP connect 与 UDP send 共四次均返回 EPERM，用户态连接登记数为零。停止后不报告 ready 或旧 program ID，按持久配置重启后重新就绪。证据为 `linux-transparent-monitor-kernel{7,612}-2026-11-02.json`。

7.0 同时重新通过正常三代际和 DNS 共存回退两项，相关 Linux/macOS Clippy、格式/diff 通过。下一批为保持暂停边界的有限恢复、API/CLI/MPA；LAN、SIGKILL 与 x86_64 仍待完成，完整发行验收 ready 不据此宣称通过。
