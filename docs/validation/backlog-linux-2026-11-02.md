# 余下 Issue 组合验收：Linux 最终内核矩阵

编译输入固定为 `862ddad624403e6f81bc994a7fe262ac04b7fb01`，包含 #23、#24、#25。共享 18 页 Web ID 为 `7c94170ee16c7b9c2f8e0aa08c219e172c2a7b6bb4834c71b2cf96a2fb4316e1`。本页随后的文档提交不改变该编译来源。Mac、Native 与本地 Mac 候选记录见[独立报告](backlog-macos-2026-11-02.md)。

[最终机器聚合](backlog-kernel-2026-11-02/acceptance.json)记录 52 个真实 QEMU 内核案例：aarch64、x86_64 分别在 6.12.115 和 7.2.14 各运行 12 个 core 与 1 个 nonlocal 案例。每个案例都核对精确匹配、退出码、实际 kernel 和串口结构化证据。四组使用相同 ABI4 对象 SHA-256 `fa5449609f86a6b50300a83a6e10d2f2150aa42eaa6676c8e64dd1581bbd5286`，原 ABI3 拒绝夹具保持独立。

此次为现有 transparent supervisor 的三条路径增加真实诊断断言，没有增加数据面操作。每个架构/内核都有三条记录，共 12 条：在真实加载程序、maps 并产生四条双栈 TCP/UDP 流量之后，`linux_diagnostics` 返回的 owned program/map ID 与实际 FD 一致，八个 live counters 落在独立读取的前后值之间，ready、flow 与 active owner 事实一致；读取前后的完整规则、路由和应用控制状态不变。对应字段为 `diagnostics_owned_ids_and_live_counters_verified` 和 `diagnostics_kernel_rules_routes_and_control_unchanged`，均为 true。

普通测试与内核验收分开记录：

| 环境 | 普通 workspace | Clippy | 补充边界 |
| --- | --- | --- | --- |
| aarch64 Linux 容器 | 234 passed、30 ignored、0 failed | all-targets `-D warnings` 通过 | ignored 不计入真实内核通过 |
| x86_64 仿真 userland 容器 | 233 passed、30 ignored、1 条运行时 skip | all-targets `-D warnings` 通过 | 下述同一 ELF 的双真实内核补验独立记录 |

x86 首轮普通测试在未修改的 `transparent::tests::udp_recovers_actual_dual_stack_addresses_and_ports_and_rejects_partial_payload` 上收到 `setsockopt` errno 92（Protocol not available）。原失败日志保留在 [userland-failure.txt](backlog-kernel-2026-11-02/net-socket-gap/userland-failure.txt)。同一未剥离 libtest ELF（SHA-256 `91721bd9c012b3f495ed2f80ac74677ef6479543592ea7cf07cdf6902b5fb857`）在两个完整 x86 内核中均精确 1 passed、0 ignored，报告在 [net-socket-gap](backlog-kernel-2026-11-02/net-socket-gap/userland-failure.json)。这支持环境相关 socket 选项转换差异的判断；没有修改产品源码、添加 ignore 或将 userland 失败改记通过。后续普通 workspace 只运行时跳过该精确名称，两个真实内核补验不混入标准 52。

x86 标准矩阵开始时曾因本地 kernel image tag 缺失而在 fixture 层退出，尚未执行案例。原短日志与[恢复元数据](backlog-kernel-2026-11-02/x86_64/fixture-image-recovery.json)保留。相同 Dockerfile 与参数从本地 BuildKit 缓存重建镜像后，核对 APK 摘要与此前 Net 串口证据一致，再重新执行完整矩阵。该 fixture 失败不计为案例失败或通过。

所有源码以只读冻结快照挂载，双架构 cargo/target 输出分开；实际网络/eBPF 操作仅在自有完整 QEMU guest 内执行。没有操作宿主 Mac Wi-Fi、DNS、路由、系统代理，未启停用户服务或原有 mysql。普通容器的只读拓扑采样不代替本页真实内核证明。静态 headless 候选的打包、实际安装及发行 daemon 的额外内核案例单独记录，不把 debug test binary 的通过自动当成发行 daemon 已验收。

ARM 静态 headless 候选已用本架构执行元数据检查、打包及实际安装。包的编译来源是 `862ddad`，打包文档来源是 `34acb5d`，未允许 dirty；ELF 没有 `PT_INTERP`，运行无需 Desktop、display 或 Node。候选包 SHA-256 为 `74e0dc1bd1355e6f6693e67d7b3d2830968f01770eb82778cebde07ba6582906`。包内文件摘要、配置保留及损坏重装不改变已安装两个二进制均通过。安装二进制完成 18 页服务、独立 CLI、HTTP/CONNECT/SOCKS5、256 请求/16 workers 零失败本地负载、连接归零、4 连接预算/第五条拒绝、端口冲突拒绝及干净重启。此处同一二进制的持久状态重启/停机快照回退不冒充跨源码版本升级；Mac 的独立旧版升级验收见其报告。负载数字只描述自有本地夹具。

ARM 安装后的独立 CLI 另完成真实在线备份/摘要检查、运行锁拒绝恢复、停止后恢复保留原库、坏摘要拒绝和重新启动。[包清单与安装报告](backlog-kernel-2026-11-02/packages/aarch64/manifest.json)保留实际来源、Web ID、eBPF 对象和二进制摘要，其他原始结果位于同目录。已安装 `hpsd` SHA-256 `d6d6d7bec8e0224cb6c8ab642a08cfaf2a05403acc808cf21936c63e0931f728` 与两个完整 ARM 内核实际执行的 release daemon 相同。两条额外发行案例均精确 1 passed，确认真实 SIGKILL、外来规则保留及重启前后 TCP/UDP 转发，独立于标准 52。

可恢复的实际包、manifest、安装结果与 installed-prefix 已复制至主工作目录 `target/validation/backlog-candidate/linux-aarch64`，不依赖当前工作树的 ignored target 留存。未打 deb/Desktop Linux 包、未发布 registry 或启用实际系统服务。
