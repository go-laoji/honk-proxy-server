# T21 / Issue #26 范围与兼容契约验收

- 日期：2026-11-02（Asia/Shanghai）。
- Issue：[P2 T21 多实例与其他平台等储备范围 #26](https://github.com/go-laoji/honk-proxy-server/issues/26)。
- 基线：`423c41c`；工作分支：`feature/future-platform-contracts`。
- 计划提交：`113b619`；最终契约见 [future-platform-contracts.md](../future-platform-contracts.md)。

## 1. 验收口径

原 Issue 要求在实施前明确产品语义、平台方案、真实供应商口径和兼容性，不把 FUT-01 至 FUT-04 加入首版隐含范围。本次交付只验收这项储备工作，没有创建或运行任何未来能力的骨架接口；没有把编译、mock 或既有基础功能当作未来能力已运行的证据。

T17 / Issue #22 的已有 [发行候选记录](release-packaging-2026-11-02.md)作为继承基线，其 macOS 网络写入未验收、Windows 未实测等限制保持原意。本任务不重复计算或扩大该证据。T19 / Issue #24 的协议/订阅/有界 Score 由独立分支负责；本文明确区分其当前任务与 T21 未来模型扩展。

## 2. 逐项交付核对

| 编号 | 已形成的可评审内容 | 本次未宣称通过的运行能力 |
| --- | --- | --- |
| FUT-01 | 稳定身份与运行 epoch 区分；显式登记的发现边界；单写状态/节点/预算归属；逐目标配置分发、部分成功与未知结果；会话不迁移 | 控制器、集群原子提交、跨机租约/会话、账户全局预算 |
| FUT-02 | Mac Transparent Proxy 与 Packet Tunnel/DNS 的官方边界；Windows TUN/WFP 选择；Rust/签名/驱动前提；共享引擎、失败语义与真实隔离验收 | Mac/Windows 捕获、DNS 全局覆盖、驱动安装/签名可用性或持久 kill switch |
| FUT-03 | IP 额度/周期与金额/余额/估算区分；真实 Gateway 可提供/不可提供的数据；观测/计费事件/对账契约；供应商输入与脱敏证据前提 | 付费账户调用、实际价格、真实账单对账、成本分摊或购买流程 |
| FUT-04 | 第三方规则格式子集及更新信任/事务/过期策略；T19 与复杂模型界线；FakeIP 地址、TTL、DNS、规则、重启与多实例兼容 | 自动外部规则更新、任意模型执行、目标 FakeIP 或数据面兼容验证 |

## 3. 与实际代码核对

本次只读核对下列代码，不修改运行行为：

- [`api::AppState`](../../crates/api/src/lib.rs)、[`storage::Store::open`](../../crates/storage/src/lib.rs) 与 [`ServiceStatus` / `Capabilities`](../../crates/domain/src/lib.rs)：`instance_id` 为本次运行/event epoch，不能当稳定集群 ID；能力反映当前目标。
- [`profiles`](../../crates/system/src/profiles.rs)、[Web transport](../../web/console/src/platform.ts) 与 [Native events](../../src-tauri/src/events.rs)：档案是连接目标，原生上下文按 target/document generation 隔离，不是编排控制器。
- [`ProviderClient::diagnostic`](../../crates/provider/src/lib.rs)、[供应商配置](../../crates/config/src/provider.rs) 与 [补池预算](../replenishment-usage.md)：现有余额诊断单位 `ips`；本地请求/节点预算与供应商真实计费周期不同，结果不确定不会自动成为免费。
- [`RuleDocument`](../../crates/config/src/routing.rs)、[`FlowOrigin` / `ProcessIdentity`](../../crates/domain/src/routing.rs)、[选择配置](../../crates/config/src/selection.rs)：本项目规则格式与 Linux 进程来源均有明确范围，新平台/格式不能静默复用未知枚举。
- [DNS 配置](../../crates/config/src/dns.rs)与 [DNS 用法](../dns-usage.md)：当前是实际地址 DNS，Linux DNS 对端 alias 不是目标 FakeIP，bootstrap/provider/health/business 的解析用途已区分。

## 4. 官方资料核查

2026-11-02 只读查阅以下第一方资料；技术建议与资料事实已在主文分开表达，没有访问供应商账户或运行平台配置写入。

| 来源 | 核查结果 |
| --- | --- |
| [Apple TN3120](https://developer.apple.com/documentation/technotes/tn3120-expected-use-cases-for-network-extension-packet-tunnel-providers) | Packet Tunnel 的建议用途是隧道；选流后代理等用途推荐其他 NE provider，不能套用成当前代理的全局捕获承诺 |
| [Apple TN3134](https://developer.apple.com/documentation/technotes/tn3134-network-extension-provider-deployment) | Transparent Proxy 可按 system extension 部署；app/system extension 生命周期不同，直接分发与最低 OS 须按 provider 表确认 |
| [Apple NETransparentProxyProvider](https://developer.apple.com/documentation/networkextension/netransparentproxyprovider) | `NO` 返回会直接放行；自身 DNS/proxy settings 被忽略，按名称连接不跳过解析 |
| [Microsoft WFP redirection](https://learn.microsoft.com/en-us/windows-hardware/drivers/network/using-bind-or-connect-redirection) | callout、原目标、redirect records/context 和代理出站防循环共同参与，不是普通 listener 就能完成 |
| [Windows Driver Policy](https://support.microsoft.com/en-us/windows/hardware/drivers/the-windows-driver-policy) / [签名要求](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-requirements--windows-vista-and-later-) | 驱动签名/系统策略是发行前提；签名本身不替代当前目标 Windows 的实测，虚拟驱动也须遵守要求 |
| [Wintun](https://www.wintun.net/) | 是 L3 包适配器，官方分发签名 DLL；源码许可与官方预编译包许可须分开核实 |
| [快代理 getipbalance](https://www.kuaidaili.cn/doc/api/getipbalance/) | 按 IP 付费的剩余提取总量与集中提取型今日剩余量是不同周期，均非金额 |
| [快代理 Gateway 指南](https://www.kuaidaili.cn/doc/using/cli_api_guide/) | 账户资金、订单配额及 24 小时统计有不同数据口径，现有订单提取适配器未接入 Gateway |
| [sing-box rule-set](https://sing-box.sagernet.org/configuration/rule-set/) / [mihomo rule-provider](https://wiki.metacubex.one/config/rule-providers/) | source/binary 与 behavior/format 各有明确语义，不能用一个“兼容第三方规则”笼统承诺 |
| [RFC 4035 §3.2.3](https://www.rfc-editor.org/rfc/rfc4035.html#section-3.2.3) | AD 是 DNSSEC 语义，合成目标不能冒充经验证的原始答案 |

Apple 页面正文由其官方 `.md` 版本读取，网页检索器仅返回 JavaScript 提示；快代理 `.com` 页面抓取超时后读取同一供应商的 `.cn` 官方帮助中心。没有使用第三方摘要替代上述资料。

## 5. 实际执行的检查

检查仅针对本任务三个 Markdown 文件；不运行会产生 OS 写入的测试、不运行 Daemon/Desktop、不抓取收费 API。

- `git diff --check` / `git diff --cached --check`：通过。
- 用 Python 标准库逐项解析三个 Markdown 文件：**29** 个相对链接均存在且位于本 worktree，记录 **14** 个唯一外部 URL；外部资料内容核对见上表。
- 用 Unicode 类别检查不可见格式字符；正文不包含意外零宽字符。
- 对比 `423c41c` 的变更路径：仅本计划、主契约和本验收记录；源码、配置、schema、依赖、中央进度和历史验收均未修改。

文档变更不增加运行行为；没有为这些可逆文档新增镜像式测试，也没有重跑历史网络/数据面矩阵。未来运行验收条件已写入每项 FUT，当前没有待补的实现伪装成已通过。

## 6. 宿主与提交范围

所有命令显式位于专用 worktree，未操作 root 或其他工作区；未启动/重启用户旧服务、覆盖 Desktop bundle、修改偏好或用户未跟踪文件。未读取/保存宿主网络配置或恢复副本，未写入 Mac 网络配置，未运行已作废的 ignored 测试。无 push、feature finish 或 Issue 关闭操作；统一验收/进度更新由主任务按依赖顺序完成。
