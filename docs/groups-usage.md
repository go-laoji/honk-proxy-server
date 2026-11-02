# 节点分组、固定与选点

HTTP、HTTPS 与 SOCKS5 TCP 入口共用 Rust 选择器。配置位于 `proxy.selection`，无分组配置时保持全池轮询。分组不会跳过已有健康、节点启停、来源指纹和租约寿命准入，也不会在没有合格节点时隐式直连。

## 配置

下面配置引用已有静态节点 `static-a`、`static-b` 和已有供应商 `vendor`，需要先在同一份配置定义它们。

```toml
[proxy.selection]
default_group = "browser"
max_sessions = 10000

[proxy.selection.inbound_groups]
browser-http = "browser"

[proxy.selection.node_tags]
static-a = ["preferred"]

[proxy.selection.provider_tags]
vendor = ["preferred"]

[[proxy.selection.groups]]
id = "browser"
strategy = "round_robin"
include_all = false
members = ["static-a", "static-b"]
providers = ["vendor"]
tags = ["preferred"]
protocols = ["http", "https", "socks5"]

[proxy.selection.groups.session]
key = "source_destination"
ttl_secs = 60
```

`members` 与 `providers` 取并集，再要求所有指定 `tags` 均匹配，并应用协议过滤。节点和所属来源的标签合并；协议列表为空表示不限。`include_all=true` 明确选择全池作为成员来源。空成员条件不会自动当作全池，空组允许保存并明确返回无候选。

静态节点、供应商、入口和默认组引用会验证；动态租约 ID 使用 `lease:42` 形式，租约消失后仍能解释显式成员的 `missing_or_retired`。删除被默认组、入口或后续路由引用的分组前，需要先移除引用。ID 是配置标识，不是账号或权限主体。

## 策略和固定

| 策略 | 选择语义 |
| --- | --- |
| `fixed` | 必须配置 `pinned_node`；优先该节点，失败只用显式回退列表 |
| `failover` | 按显式成员顺序选择当前合格节点，其余来源成员按池顺序；没有合格者则失败 |
| `latency` | 选择当前健康策略下有新鲜主动探测延迟的合格节点；没有证据不会虚构分数 |
| `round_robin` | 在当前合格成员间轮询，候选集合变化后按新集合分配 |
| `weighted` | 按 `weights = { static-a = 1, static-b = 3 }` 的整数比例轮询，未指定节点权重为 1 |

任何策略都可设置 `pinned_node`。固定优先于策略和已有会话；固定节点不合格时，只允许 `fallback_nodes` 中且仍属于当前组、满足过滤和准入的节点，绝不扩大到其他成员或全池。固定/故障切换都是选点前根据已有事实作决定；握手失败不会立刻把同一业务请求重放给另一个节点。真实失败进入统一健康反馈，影响后续选点。

取消固定同时清除回退列表；原策略是 `fixed` 时切换为 `round_robin`。`failover`/`fixed` 回退优先按明确回退列表顺序，其余策略在允许范围内继续按自身规则分配。

当前 HTTP/HTTPS/SOCKS5 上游均只提供 TCP 数据通道，UDP 模拟会逐节点返回 `tcp_only`，不继承 TCP 探测成功。UDP 数据通道另由 T15/T19 接续。

## 会话和配置代际

`session.key` 支持 `source_ip` 或 `source_destination`（源 IP + 目的主机/端口）；两者均隔离入口和分组。它描述流量来源，不是用户登录。共享 NAT 的客户端可能共享源 IP，会话规则应按实际网络选择。

TTL 为 1–86400 秒，实际绑定截止时间不超过节点剩余可准入寿命，扣除服务/来源要求的最小剩余寿命。复用前仍检查健康、启停、来源、成员范围和协议，不通过缓存固定引用延长租约。相同绑定的命中不刷新 TTL。系统时钟回拨不能延长本进程中既有静态或动态节点的单调截止时间；显式修改静态节点的到期配置属于新的配置意图。

会话表每代际最多 `max_sessions` 条（1–100000）；到期记录定期按需回收，容量满时新流量仍可按策略选点，但诊断显示未能建立新会话。配置切换和重启会重建会话；旧引擎排空时立即清理绑定并禁止继续扩张旧会话表，避免反复热更叠加会话内存。

已建立的连接保留自己的 socket 和配置代际，不承诺迁移、切换出口或在供应商到期后保活。旧客户端 TCP 会话排空期间仍使用原代际，配置影响新接入的连接。

## CLI 和 API

```sh
hps group show
hps group simulate --group browser --source-ip 192.0.2.10 --host example.com --port 443
hps group simulate --inbound browser-http --udp
hps group simulate --group browser --limit 50 --after 'lease:42'
hps group upsert group.toml --expected-revision 3
hps group pin browser static-a --fallback static-b --expected-revision 4
hps group unpin browser --expected-revision 5
hps group save selection.toml --expected-revision 6
hps group delete browser --expected-revision 7
```

`group.toml` 从单个分组根开始，例如 `id="browser"`、`include_all=true`；`selection.toml` 从整个选择策略根开始，不包含 `proxy.selection` 外层。CRUD 和固定均生成配置版本、先准备再激活，并要求 `expected_revision`；冲突退出码为 7。CLI 无需桌面。

| 方法 | 路径 | 内容 |
| --- | --- | --- |
| GET | `/api/v1/groups` | 运行版本、策略、最近实际选点和默认选择预览 |
| POST | `/api/v1/groups/simulate` | 分组/入口/来源/目标/传输上下文及 `after`/`limit` |
| POST | `/api/v1/groups/save` | `{configuration,expected_revision}` 整体策略 |
| POST | `/api/v1/groups/upsert` | `{group,expected_revision}` 创建或更新 |
| POST | `/api/v1/groups/delete` | `{group_id,expected_revision}`，保留引用校验 |
| POST | `/api/v1/groups/pin` | `{group_id,node_id,fallback_nodes,expected_revision}`；null 取消固定 |

模拟只读，不连接节点、不请求供应商、不建立或刷新会话、不推进轮询。候选分页最多 200 条，选择决定基于整个候选集合，与当前页无关。结果解释成员、标签、协议、固定范围、健康、寿命和缺失原因；延迟只显示已有新鲜证据。最近实际选点记录的是拨号尝试前的决定，不保证连接成功，不伪造业务站点状态。

`/console/groups.html` 提供独立 MPA 的分组 CRUD、选择策略/入口绑定/标签编辑、候选模拟与分页、手动固定和版本冲突反馈。Desktop 共用页面，通过原生桥访问相同接口；所有可达使用者直接访问全部功能，无项目认证。嵌套组和高级 Score 由 T19 交付，目标规则由 T11 复用选择器。

配置限制：最多 256 组；每组最多 10000 个显式成员、64 个来源、1000 个回退节点、10000 个权重项；权重 1–1000，全部组的成员/回退/权重引用合计最多 100000。标签每处最多 32 个；API 和 CLI 输入大小也有上限。
