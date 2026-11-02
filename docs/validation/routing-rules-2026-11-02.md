# T11 显式规则路由与候选验收

对应 [Issue #16](https://github.com/go-laoji/honk-proxy-server/issues/16)。本记录覆盖规则匹配、实际显式代理、候选/激活、CLI 与共享 MPA。Linux 本机进程来源、内核与用户态原子规则版本仍需与 T15 联合交付；Issue 保持 open。

## 已通过验证

| 范围 | 结果 |
| --- | --- |
| macOS aarch64 Rust 工作区 | 122 passed，0 failed，含 doc-tests |
| Linux aarch64 Rust 工作区 | 121 passed，0 failed；4 个需特权的 DNS 入口默认 ignored，既有实际 eBPF DNS 证据另见 T09 |
| clippy / fmt | 全工作区 all-targets `-D warnings` 及格式通过 |
| React / 18 个 HTML 入口 | 类型检查与生产构建通过，共用组件和资源清单 |
| Chromium 完整回归 | 14 passed，包含候选/排序/激活和实际业务 HTTP 字节 |
| CLI | 真实独立 Daemon 上 CRUD、启停、排序、运行/候选模拟、显式 apply、过期版本退出码 7 通过 |
| Native bridge | 独立构建及实际窗口候选保存/模拟/显式激活和真实流量通过 |

新增引擎测试在 HTTP、HTTPS、SOCKS5 三入口使用同一规则顺序，实际经过指定节点/组或明确直连访问本地 HTTP/HTTPS 目标，检查字节返回；拒绝和直连阶段没有代理池采购需求。指定节点不可用不改用无关健康节点，全匹配拒绝会遮住默认直连。Direct 使用共享 business DNS，保留 HTTP Host；原有 3×3 出入口协议、取消和不重放测试继续通过。

匹配测试覆盖域名标签边界与大小写、端口/来源/入口组合、IPv4/IPv6 和 mapped IPv4、只读 DNS 观测推断的默认关闭/唯一/歧义、未知上下文以及顺序变化。无已验证进程来源和无 UDP 数据通道时，相关启用配置拒绝预备；不猜远端进程。

真实 HTTP API/client 测试验证候选保存不改变 head、跨候选编辑/启停/排序/删除、运行与候选模拟不同、并发版本冲突、失效节点在激活前重新验证及失败历史，以及混合配置候选需明确基于当前版本另存规则。模拟有独立并发上限，工作持有许可直到完成，满额拒绝后可恢复，不产生生产需求。

补池范围测试使用本地真实 HTTP 契约服务，不是付费供应商账号调用：只有组允许且协议/来源标签匹配的供应商收到提取；指定节点、无关来源、停止入口和切到直连后的旧策略信号不触发购买。快代理未采购的边界不变。

端到端发现并修复手动入口状态在规则热更新时丢失的问题：`enabled` 未变化时保留手动启停覆盖及端口；显式修改 `enabled` 清除覆盖，重启按持久配置恢复。单独的持久重启测试与浏览器同端口实际流量都通过。

## Desktop 实际验收与 Linux 剩余门槛

此前 DNS/分组的独立 `HPS DNS Validation.app` 与 17995 验收实例及 `HPS Feature Validation.app`（`dev.hps.feature-validation`、17996）均已构建。后者 CLI 与服务确认共享 build ID 为 `f9c83fae4ff7d758bcd5de2c4cb9676b4ab988ecac6715effe7cb43d1804979a`，当时窗口受锁屏阻挡；后续实际验收如下。

2026-11-02 解锁后，在 `HPS Connection Validation.app` 连接 17999 隔离服务，窗口新建启用规则 native-reject-fixture，仅匹配目标端口 56529，动作 reject。保存后明确显示“候选 r5 / 运行 r4”；真实 CONNECT 到 56529 仍返回 200。窗口模拟候选显示 r5 首条命中该规则、动作拒绝且没有副作用。点击显式激活后显示 applied / 实际 r5，新 CONNECT 返回 403，已经建立的 56528 隧道继续回显；后续连接详情仍显示原 r4。原生窗口门槛已满足，原用户 17900 未变动。

`local_process_available=false`、`kernel_rule_revision=null`、`kernel_rules_ready=false` 都是当前实际状态。T15 必须提供真实 Linux 本机进程来源、透明流量上下文和原子规则代际证明后，才能关闭 T11。DNS 捕获成功不作为这些规则能力的替代证明。

T14 已统一配置预备与规则诊断等任务的服务所有权。受控阻塞预检、调用方取消、工作容量和关闭等待回归通过，客户端离开不会提前释放仍在执行的操作锁或许可。

## T15 联合门槛完成

2026-11-02，aarch64/x86_64 × Linux 6.12.115/7.2.14 的 48 项核心矩阵与 4 项非本机目标补充全部通过。实际 cgroup 进程条件、首条规则/固定组、候选模拟与实际决策、immutable 内核代际、热更旧流保持/新流拒绝、发布失败回退均有真实数据面证据。规则能力现取自实际范围、接口、links、routes、policy 和用户态引擎，暂停/停止/不支持平台不误报进程与 UDP 就绪。原有本节之前的 false/null 是当时的历史状态，现以 [联合验收](linux-transparent-acceptance-2026-11-02.json) 为准。
