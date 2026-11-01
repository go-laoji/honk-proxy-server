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
- 测试只操作自有 Docker/QEMU/netns/cgroup/临时服务；保留 Mac 原应用、17900/yun-free 状态以及原始未跟踪文件。开发分批提交，不 push、merge 或执行 feature finish。
