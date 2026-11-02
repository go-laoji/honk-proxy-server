# SOCKS5 UDP、标准订阅与高级分组

## SOCKS5 UDP ASSOCIATE

SOCKS5 入站支持无认证 TCP CONNECT 与 UDP ASSOCIATE，BIND 仍返回 REP=7。UDP 关联必须通过实际 TCP 控制连接建立，回复 relay 地址使用该连接真实本地 IP 与独立 UDP 端口。TCP EOF、控制数据、入口停止、Daemon 停止或空闲超时会释放 UDP socket、目标连接及资源额度。Mac 开发验收仅使用回环 socket，不修改系统代理、Wi-Fi、DNS 或路由。

按 [RFC 1928](https://www.rfc-editor.org/rfc/rfc1928) 约束来源：请求地址为非零 IP 时必须等于 TCP peer IP；未指定端口时首个有效 UDP 包锁定实际来源端口，明确端口时严格校验。不同 IP/端口、RSV 非零、FRAG 非零、畸形地址和超过 65507 字节的 wire datagram 直接丢弃。关联请求的来源地址接受 IPv4/IPv6；UDP 包的目标地址支持域名、IPv4/IPv6。域名使用现有 Business resolver，不在握手时解析目标或购买节点。

每个关联最多 32 个目标，每个目标队列最多 8 包。TCP owner 和每个目标分别占用现有全局/入口连接额度；满额/满队列丢包，UDP 不重传。当前业务 payload 上限为 65245 字节，为最长域名头预留空间。目标流复用路由、节点操作、健康门槛、租约、会话、带宽、连接观测及主动断开。管理断开单个 UDP 目标不会关闭其他目标；TCP owner 关闭释放全部目标。

UDP 只选择 SOCKS5 潜在出口或明确的 Direct 规则；HTTP/HTTPS 候选标记 `tcp_only`，不通过 CONNECT 冒充 UDP。没有潜在 UDP 路径时 association 返回 REP=7；目标规则拒绝或上游失败时不回复成功业务数据，也不隐式直连。上游 SOCKS5 仍需真实完成 UDP ASSOCIATE；已有 TCP 健康证据不被宣称为 UDP 健康探测，主动 UDP 探测仍未支持。`capabilities.socks5_udp` 单独报告协议实现能力，不代表某个供应商已支持 UDP 或已通过端到端探测。

## 独立标准订阅

短效提取 API 仍是主线。`subscriptions` 是独立的标准订阅定义与成功快照，沿用 SQLite 配置 revision/CAS、激活和回滚，不新增 lease/batch/extraction task，不参与供应商补池/额度采购。来源标识为 `subscription:<id>`，节点标识为 `sub:<id>:<24位摘要>`；摘要包含实际端点与凭据，显示名称和刷新时间不改变节点身份。

```toml
[[subscriptions]]
id = "office"
name = "办公室标准订阅"
url = "https://subscription.example/list?token=replace-locally"
format = "uri" # 或 clash
max_nodes = 1000
timeout_ms = 10000
enabled = true
```

成功 GET/文档导入原子替换该来源快照，不改其他订阅或短效租约。URL/格式变化清除旧来源快照，停用保留快照但停止新准入，既有连接自然排空。失败、空文档、重复端点、畸形内容、不支持协议/字段、超限或过期 revision 都不会清空现有快照。删除有组来源引用时先移除引用。没有后台定时刷新；使用来源页、CLI 或 API 显式刷新。

`fetched_at_unix` 仅表示收到快照的时间，`origin` 区分远程 GET / 本地文档导入；**不是供应商有效期**。支持的标准格式没有明确节点到期契约，`expires_at_unix` 始终为 null，页面显示“有效期未知”。不以本地缓存时限冒充供应商实际有效期；需要有期限的节点应继续使用短效 API 租约模型。

支持格式：

- URI：每行 HTTP、HTTPS、SOCKS5/SOCKS5H URL；支持端口、百分号编码凭据、IPv6、显示名称 fragment；可接受整文标准或 URL-safe base64（含无 padding）；空行/整行注释忽略。
- Clash：读取 YAML/JSON 顶层 `proxies`。支持 `type: http`（`tls: true` 对应 HTTPS）及 `type: socks5`，字段 `name/server/port/username/password`；HTTPS 可配置 `sni`，SOCKS5 可配置 `udp: true`。其余顶层配置不会被执行或导入。
- 不支持 Shadowsocks、VMess/VLESS、Trojan、SOCKS over TLS、自定义 headers、dialer-proxy、跳过证书校验、tfo/mptcp、指定 IP 版本等字段；为避免静默降级，整份刷新明确拒绝。SOCKS5 `udp: false` 也拒绝，当前快照端点模型不承诺导入每节点 TCP-only SOCKS 限制。

Clash 字段按 [Mihomo 官方 HTTP](https://wiki.metacubex.one/en/config/proxies/http/) 与 [SOCKS](https://wiki.metacubex.one/en/config/proxies/socks/) 文档验证；YAML 使用 [serde_yaml_ng 0.10](https://docs.rs/serde_yaml_ng/0.10.0/serde_yaml_ng/) 的 typed serde 解析与未知字段拒绝，不执行远程 include 或自定义代码。HTTP GET 不跟随重定向、不隐式重试、不继承系统代理，使用现有 Provider resolver。文档最多 1 MiB，单来源最多 10000 节点，最多 32 来源且全量快照最多 20000 节点；SQLite 配置文档的原有 2 MiB 持久预算继续生效。动态发布池总上限为 120000（短效租约最多 100000 + 独立订阅快照最多 20000），订阅不挤占或误删已有租约。

URL 可能包含 token，API/CLI/配置导出一律将整个 URL 脱敏，上游凭据分别脱敏，调试不输出 URL。定义编辑保留 `[REDACTED]` 可复用旧 URL；快照凭据由 GET/导入拥有，来源保存不能注入快照。管理面无需账号；具有访问管理 API 能力的调用者仍可保存配置。

```sh
hps --output json subscription list
hps subscription save office.toml --expected-revision 1
hps subscription refresh office --expected-revision 2
hps subscription refresh office --expected-revision 3 --document proxies.yaml
hps subscription delete office --expected-revision 4
```

API 为 `GET /api/v1/subscriptions`、`POST /api/v1/subscriptions/{id}/save|delete|refresh`；save 接受 `configuration` 与 `expected_revision`，delete 接受 `expected_revision`，refresh 接受 `expected_revision` 与可选 `document`（null 表示远程 GET）。Rust typed client 与共享 React transport/Native whitelist 使用同一契约。来源页承载订阅区，18 个 MPA HTML 入口保持不变。

## 嵌套组与 Score

组的 `groups = ["child"]` 表示展开子组的合格候选并集，和 `members/providers` 并集后继续应用父组的过滤。子组的成员、标签、协议和固定/fallback 范围不被父组绕过；子组固定节点合格时只向父组提供该节点，失效时只提供仍合格的显式 fallback。非固定子组提供候选集合，最终策略/会话属于父组，不推进子组轮询或建立隐藏子组会话。候选 `member_groups` 列出贡献候选的直接子组。

配置预备阶段拒绝未知子组、重复子组、循环、超过 8 层及全策略 4096 次递归展开预算；父子都不能绕过协议/健康/租约等准入。模拟与生产使用同一个选择器。

```toml
[[proxy.selection.groups]]
id = "office-udp"
providers = ["subscription:office"]
protocols = ["socks5"]

[[proxy.selection.groups]]
id = "best"
groups = ["office-udp"]
strategy = "score"

[proxy.selection.groups.score]
latency_weight = 60
lifetime_weight = 30
priority_weight = 10
latency_ceiling_ms = 2000
lifetime_target_secs = 300
missing_evidence_penalty = 250
```

Score 只在合格候选之间比较，权重各为 0–1000 且总和非零，分量归一到 0–1000：

| 分量来源 | 归一规则 | 无证据 |
| --- | --- | --- |
| 当前健康策略的新鲜主动探测延迟 | `1000 - min(ms/latency_ceiling_ms, 1)*1000` | 未探测/过期时未知 |
| 实际准入剩余寿命 | `min(remaining_secs/lifetime_target_secs, 1)*1000` | 标准订阅/无到期配置未知 |
| 显式运营优先级 `weights[node_id]` | 直接使用 1–1000 | 未配置未知，不能用默认权重冒充证据 |

每个启用分量贡献 `floor(normalized*weight/总权重)`；缺证据贡献为负的 `missing_evidence_penalty`。总分为所有贡献之和（可为负）。同分保持当前明确成员/候选顺序。来源、原始观察、归一值、权重、缺证据扣分和贡献通过 typed DTO/CLI/组页面返回。没有成本或地域数据时不编造成本、地域或供应商品质；`weights` 是运营输入，绝不是供应商价格。
