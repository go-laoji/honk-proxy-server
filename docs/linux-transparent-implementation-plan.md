# T15 Linux 透明数据面实施计划

对应 [Issue #20](https://github.com/go-laoji/honk-proxy-server/issues/20)，分支 `feature/linux-transparent`，基于 T14。与 T11 #16、T13 #18 的 Linux 进程/内核代际门槛联合交付；三项未联合验证前保持开放。仅学习 honk 的能力，不集成其实现或对象。

## 当前事实与技术选择

现有自研 cgroup connect4/6 观察/拒绝程序和八个 DNS 钩子已在实际 6.12.115、7.0.14 内核验证；DNS 使用解析器别名，不能直接充当任意目标的透明业务入口。业务引擎已有完整 TCP 拨号、固定/规则/健康/租约、真实连接登记与断开，但没有业务 UDP 通道或网关捕获。`local_process_available=false`、`kernel_rule_revision=null` 和完整 datapath_ready=false 仍是正确状态。

业务捕获采用自研 Rust **cgroup + TCX + transparent socket**，保留原目标地址/端口。cgroup 标记选择的本机 socket 并记录可信进程元数据；lo 的 TCX egress 将元数据关联到具体网络元组，TCX ingress 处理本机及明确选定的 LAN 网卡，通过 `bpf_sk_assign` 交给专用 TCP/UDP socket。策略路由仅匹配本项目独占标记，在私有表送往本地。用户态使用原目标接入同一选路引擎，不修改 HTTPS 正文，不默认直连。

这个组合是本项目的设计选择，不是已经通过的内核结论。依据包括 [Linux transparent socket 与策略路由说明](https://cdn.kernel.org/doc/html/latest/networking/tproxy.html)、[Linux 6.12 socket assign 自测](https://raw.githubusercontent.com/torvalds/linux/v6.12/tools/testing/selftests/bpf/progs/test_sk_assign.c) 和 [Aya 0.14 TCX 生命周期](https://docs.rs/aya/latest/aya/programs/tc/struct.SchedClassifier.html)。具体 helper 可用性、socket cookie 传播与双栈行为由实际内核逐步验证；不把文档最低版本当作本项目已支持的版本。

优先使用 TCX 的独立 FD link，退出时仅释放自有 link，不改动其他程序或删除整个 clsact。目标验证内核均支持 TCX；本轮不加入未实测的旧内核兼容路径。路由/规则通过 Rust netlink 管理，不把执行 shell 中的 iptables/nft 脚本当作后端实现。

## 分批开发与提交

1. **契约与隔离边界**：独立 Linux 捕获配置、明确本机 cgroup 与 LAN 网卡选择、CIDR bypass、容量/超时、私有 mark/table/priority、版本化无隐式填充 ABI 和实际状态 DTO。未启用时不创建任何内核对象；非 Linux 明确拒绝启用，不能静默忽略。
2. **本机 TCP 最小真实路径**：cgroup socket 标记/排除 Daemon、TCX 元组与进程关联、透明监听和精确目标恢复，接入现有选路/节点/健康/资源/流量事实源。独立内核测试使用本地受控 HTTP/HTTPS/SOCKS5 上游和隔离目标，必须有实际字节与指定出口证据。
3. **网关和 UDP**：明确 LAN 接口的入站捕获，不猜网关后进程；UDP 原目标 ancillary、透明回复 socket、有界关联/排队/数据报尺寸/空闲清理。TCP-only 节点仍被排除；显式 Direct UDP 和 SOCKS5 UDP 出站按真实能力分别验证。T19 的 SOCKS5 UDP 客户端入口与高级订阅保留后续范围，共用本轮 UDP 出站基础。
4. **原子代际与热更**：内核流元数据记录实际策略代际。用户态保留对应引擎快照，新流使用新代际，已存在 TCP/UDP 关联保持旧代际；不能用当前配置冒充流创建时规则。内核 map 更新、持久 CAS、用户态发布纳入明确互斥/失败回退，旧配置可继续服务；捕获身份/绑定变化需要重启时明确返回。
5. **对象与运行恢复**：网络命名空间/配置身份锁、精确对象标识与归属、异常退出残留回收；私有 route/rule 冲突拒绝接管，不能 replace/delete 他人对象。监听网卡/地址/路由变化并校验真实 link/route 存在性；丢失时降级状态并有限重试，不假报 ready。正常停止先关闭新捕获再回收服务，关闭/失败都等待自有任务。
6. **控制面闭环**：实际状态/范围/错误/计数/程序及 map ID、真实内核与用户态版本、启停/重新检查的 API、CLI 与共用 Linux MPA。操作由 T14 任务/事件记录，无项目账号或权限层。窗口不以 root 运行，特权 Linux 操作只在目标 Daemon。
7. **联合验证与文档**：实际 aarch64/x86_64、6.12 LTS/当前可用内核矩阵，双栈本机/LAN、TCP/UDP、DNS 共存、进程/组/拒绝/直连、热更保持旧流、bind/route/map/link 失败与 SIGKILL 后恢复。完成后按依赖顺序回复并关闭 T11/T13/T15；普通单测的 ignored 不算内核验收。

## 必须保持的语义

- 原始目标与可信进程只来自自有内核元数据；显式代理远端客户端、LAN 客户端进程始终未知。cookie/元组/命名空间不匹配或必要元数据丢失时拒绝受保护的新流，不跳过进程规则而默认放行。
- eBPF 仅负责接管/元数据/代际；完整业务规则、供应商租约和选点仍共享 Rust 引擎。健康探测、供应商 API、DNS/bootstrap 与 Daemon 出站要明确 bypass，不能再次进入捕获或触发无限补池。
- 外部程序已有 mark/link/route 需保持。默认捕获范围显式、默认关闭，bypass 优先；协议或报文格式不支持时区分旁路范围和已选择接管后拒绝，不能在“已接管”状态下静默直出。
- IPv4 分片、IPv6 扩展头、GSO/GRO、TCP 已建立 socket 查找、UDP 无连接/多目标与异常数据报各有明确处理和上限。首轮可明确拒绝未实现形态，但文档和计数不能假称已支持。
- UDP 不重传业务数据，不把 TCP CONNECT 当作 UDP 能力，不把 HTTP 供应商猜成可转发 UDP。只在 SOCKS5 实际成功建立 association 后声称相应出口可用。
- 所有 map、关联、跟踪记录、工作队列和等待都有上限；完整 ready 由实际 listener、link、route、代际与可执行规则共同决定。仅对象成功 load/attach 或 DNS 可用不等于业务 ready。
- 测试只操作自有 Docker/QEMU/netns/cgroup/临时服务；保留 Mac 原应用、17900/yun-free 状态以及原始未跟踪文件。开发分批提交；按用户最新授权及 [Git Flow 收尾规则](git-flow-workflow.md) 顺序检查和 finish 已验收分支，不 push 或合入 main。

## 后续 owner 与代际联调重点

- UDP table 以完整元组及内核 flow identity（cookie/first_seen）区分重用，旧 owner 清理不得移除新 owner。回复前同步检查当前归属，已知旧 association 不能给新客户端回复。异步发送等待后要重新检查归属。
- TCP/UDP 共用 max_pending；UDP 会话、全局排队报文数与字节分别受独立 permit 限制。配置变更保留旧引擎与在途预算，不能借创建新代际重置资源额度。停止捕获只取消自身透明流，不停止普通显式代理入口。
- 内核流代际必须保持不可变；更新 last_seen 不应通过覆盖整条旧副本把已更新的 generation 覆盖回去。UDP idle 回收后下一个 association 才能选新代际。GC 与内核新包并发的缺失必须失败关闭并有计数，不能用当前引擎冒充旧代际。
- 快照回收除活跃用户态流，还要覆盖已捕获、尚未 accept 的旧代际元数据；不得仅凭显式入口 idle 就丢掉其引擎。32 个代际上限需要真正释放依据，不能无限持有或悄悄覆盖。
- 监视必须核对实际 cgroup links、TCX、接口和路由，而不只查看 program/map ID。Linux 6.12 TC 不能使用 netns cookie helper，接口移动或丢失须降级并精确 detach；不能把新命名空间的包当成原范围继续声称 ready。
- DNS 共存要旁路自有双栈 alias 前缀，尤其 IPv6 `fd77:6870:7300::/112`；守护进程出站与透明回复始终排除。LAN 来源永远不填进程；本机/LAN 元组冲突应明确拒绝而不是借用可信本机元数据。

## Daemon 接入的发布与回收约束

- 使用默认关闭的 `DaemonConfig.linux_capture`，非 Linux 解析配置后明确拒绝准备；对象/范围/mark/表/端口及预算变化需要重启，规则、组与节点选择继续热更。DNS 与业务捕获分别拥有对象，准备时补齐自有 DNS alias 旁路。
- 透明控制器持有固定准备身份；实际运行对象持有 socket、Capture、预算、每条 TCP/UDP owner、有限的代际表与受跟踪循环。停止实际运行对象并等待任务后，才丢弃 socket/link/route；再次启动重新核对实际对象身份。循环不得通过长期强引用形成控制器/任务环。
- 准备阶段保持内核策略禁用，绑定 listener 并核对自有路由，先登记候选引擎。持久 revision CAS 成功后，整份内核策略的 RCU 替换是新捕获流的代际选择点；每次按实际 flow.revision 找对应引擎，不回落到当前配置。发布前失败回退持久头并保留旧服务，发布后不把已产生的真实新流描述成旧代际。
- 捕获启用时，即使只有实例名称等设置变化，也不能仅复用旧 generation 的引擎却给内核发布新的 revision。现有显式代理的快照复用捷径需要对此分支作出限制。
- 内核身份是 insert-only，活动时间只作为近似 GC 提示。每个用户态 owner 对应一次实际 `routing_metadata` 的完整身份；按该身份取消并等待单流结束，再精确退休。UDP 第一条已属旧身份的数据报不能在用户态换成当前引擎重放，后续由应用决定是否再次发送。TCP 已结束时仍需为 FIN/ACK 留有有界安静期。
- 旧代际除了业务引擎活跃流，还要覆盖内核尚未交给 accept/recv 的身份；这些 pending 身份有有限期限，超时精确退休后才允许丢弃快照。代际预算保持 32，元数据/owner 上限沿用 max_flows；不能通过创建新预算绕过旧流占用。
- 控制面 ready 必须核对当前用户态与内核代际、实际 listener、cgroup/TCX、路由和运行对象状态。接口移动、link/route 缺失或 namespace 不符立即降级；错误恢复不能以 program ID 存在代替实际挂载。
