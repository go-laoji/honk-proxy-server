# T19 / Issue #24 实施计划

## 范围与安全边界

实现 SOCKS5 UDP ASSOCIATE、标准 URI/Clash 订阅、嵌套组与可解释 Score，复用 Rust 核心、独立 Daemon/CLI、typed client 和共享 React/Tailwind MPA。仅回环 UDP/TCP 与无 OS 写入验收；禁止宿主系统网络修改，所有运行设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`，使用独立 target/dist，不运行旧 SystemConfiguration 测试二进制。

## 持久模型

订阅是独立 `subscriptions` 配置与成功快照，随 SQLite 配置版本原子发布。来源使用 `subscription:<id>`，节点使用 `sub:<id>:<digest>`，不进入 extraction task/batch/lease 表，不触发短效补池。快照接收时间不是供应商有效期；未提供有效期的标准订阅节点 `expires_at_unix = None`，明确显示未知。刷新拒绝空、畸形或不支持协议文档，保留旧快照；URL/凭据与节点凭据一律脱敏。只接受支持的 HTTP/HTTPS/SOCKS5 URI（可整文 base64）与 Clash `proxies` 条目，不假称支持其它协议。

## 分组语义

显式 `groups` 子组取候选并集。每个子组保留成员、标签、协议、固定及 fallback 约束；父组继续过滤并应用自己的策略与会话。验证拒绝未知引用、环、超过 8 层或 4096 次展开。Score 限于新鲜探测延迟、剩余寿命、显式运营权重，输出来源、归一分量、权重、缺证据惩罚和总分；不推断供应商成本、地域或质量。

## 实施与验收

1. SOCKS5 UDP 绑定 TCP 控制连接；源 IP/端口约束；FRAG 拒绝；有界目标会话与 datagram 大小；复用路由、健康、连接观测、带宽与取消。验证 IPv4/IPv6、上游 SOCKS5 UDP、直连/拒绝规则、TCP 关闭和资源释放。
2. 独立订阅配置、受限 GET、严格解析和持久发布，补 API/client/CLI/页面。验证协议/凭据/Unicode 名称、重复/空/畸形/超限、来源隔离、失败保留及重启。
3. 嵌套组验证与生产/模拟一致选择；Score 可解释；补 CLI 与组页面。验证环/深度/预算、固定范围、缺证据惩罚、UDP 能力及租约到期。
4. focused Rust tests/clippy、前端 typecheck/build；独立端口端到端需与根代理协调。文档记录真实验证及未覆盖边界；小步 scoped commits，不 finish/push/关闭 Issue。

参考：RFC 1928；Mihomo 官方 HTTP/SOCKS 代理字段；serde_yaml_ng 官方 API 文档。订阅解析不执行 YAML 标签，不支持远程 include。
