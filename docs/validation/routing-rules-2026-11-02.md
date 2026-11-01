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
| Native bridge | 最新桥测试与独立 `.app` 构建通过，实际窗口仍待 Mac 解锁 |

新增引擎测试在 HTTP、HTTPS、SOCKS5 三入口使用同一规则顺序，实际经过指定节点/组或明确直连访问本地 HTTP/HTTPS 目标，检查字节返回；拒绝和直连阶段没有代理池采购需求。指定节点不可用不改用无关健康节点，全匹配拒绝会遮住默认直连。Direct 使用共享 business DNS，保留 HTTP Host；原有 3×3 出入口协议、取消和不重放测试继续通过。

匹配测试覆盖域名标签边界与大小写、端口/来源/入口组合、IPv4/IPv6 和 mapped IPv4、只读 DNS 观测推断的默认关闭/唯一/歧义、未知上下文以及顺序变化。无已验证进程来源和无 UDP 数据通道时，相关启用配置拒绝预备；不猜远端进程。

真实 HTTP API/client 测试验证候选保存不改变 head、跨候选编辑/启停/排序/删除、运行与候选模拟不同、并发版本冲突、失效节点在激活前重新验证及失败历史，以及混合配置候选需明确基于当前版本另存规则。模拟有独立并发上限，工作持有许可直到完成，满额拒绝后可恢复，不产生生产需求。

补池范围测试使用本地真实 HTTP 契约服务，不是付费供应商账号调用：只有组允许且协议/来源标签匹配的供应商收到提取；指定节点、无关来源、停止入口和切到直连后的旧策略信号不触发购买。快代理未采购的边界不变。

端到端发现并修复手动入口状态在规则热更新时丢失的问题：`enabled` 未变化时保留手动启停覆盖及端口；显式修改 `enabled` 清除覆盖，重启按持久配置恢复。单独的持久重启测试与浏览器同端口实际流量都通过。

## Desktop 与 Linux 剩余验收

DNS/分组的独立 `HPS DNS Validation.app` 与 17995 验收实例仍保留，用户原 `.app` / 17900 实例没有被覆盖或重启。新的 `HPS Feature Validation.app` 已使用独立 `dev.hps.feature-validation` 构建完成，对应 17996 临时验收实例。CLI 与服务确认共享 build ID 为 `f9c83fae4ff7d758bcd5de2c4cb9676b4ab988ecac6715effe7cb43d1804979a`。实际窗口仍待解锁，不能用编译代替窗口操作。

`local_process_available=false`、`kernel_rule_revision=null`、`kernel_rules_ready=false` 都是当前实际状态。T15 必须提供真实 Linux 本机进程来源、透明流量上下文和原子规则代际证明后，才能关闭 T11。DNS 捕获成功不作为这些规则能力的替代证明。

T14 还需统一配置候选预备任务的所有权：客户端取消不能让仍在运行的预备工作绕开运行队列/并发预算。规则模拟已独立处理这一边界，其他长任务会随统一任务模块接续审查。
