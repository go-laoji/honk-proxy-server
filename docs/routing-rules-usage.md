# 规则路由、候选与激活

HTTP、HTTPS 和 SOCKS5 TCP 接入使用同一个 Rust 匹配器与拨号器。配置位于 `proxy.routing`，按数组顺序首条命中；没有命中时执行 `default_action`。默认动作 `kind="default"` 沿用入口绑定组或默认全池选择，不隐式直连。

## 配置与匹配

```toml
[proxy.routing]
use_dns_observations = false

[proxy.routing.default_action]
kind = "default"

[[proxy.routing.rules]]
id = "example-via-group"
enabled = true

[proxy.routing.rules.conditions]
domain_suffixes = ["example.com"]
destination_ports = [{ start = 443, end = 443 }]
transports = ["tcp"]
inbound_protocols = ["http", "https", "socks5"]
# inbounds = ["browser-http"]
# source_cidrs = ["192.0.2.0/24"]

[proxy.routing.rules.action]
kind = "group"
id = "browser"
```

示例要求先定义 `browser` 分组；入口 ID 和静态节点引用也必须存在。最多 1024 条规则，每类列表最多 256 项，全部规则主要条件条目合计最多 100000。未知字段（包括账号条件）、错误 CIDR、端口范围和无效引用都拒绝。

条件类别之间是 AND，各列表内部是 OR；`domains` 与 `domain_suffixes` 共同组成一个域名类别，精确或后缀任一命中即可。后缀按标签边界匹配，不把 `notexample.com` 当成 `example.com`。名称忽略 ASCII 大小写和正常末尾点。

支持目的字面 IP 的 `destination_cidrs`、`destination_ports` 范围、`transports`、`inbound_protocols`、`inbounds`、`source_cidrs`。IPv4/IPv6 均支持，IPv4 CIDR 能识别 IPv4-mapped IPv6 来源。仅有域名而无字面目标 IP 时，CIDR 条件返回 `destination_ip_unknown`，不会为模拟或匹配额外发起解析。未知入口、源地址或进程不会被伪造为已匹配。

空条件匹配全部，必须按预期排序；前面的全匹配拒绝规则会遮住后面的直连/default 动作。共享页面新规则默认停用，启停、排序和编辑也先生成候选。

## 动作与租约

| `kind` | 行为 |
| --- | --- |
| `default` | 使用入口绑定组或默认选择策略 |
| `node` + `id` | 只选择该节点，仍检查健康和寿命，不改成其他节点 |
| `group` + `id` | 使用指定组，保留固定、过滤、回退和会话边界 |
| `direct` | 明确直连，域名通过共享 DNS 的 business 用途解析 |
| `reject` | 拒绝连接；HTTP 返回 403，SOCKS5 返回规则拒绝码 2 |

直连保留 HTTP Host；CONNECT/TLS 仍是端到端字节流，不解密业务 HTTPS。指定节点/组失败不自动直连，不重放业务请求。拒绝与直连不产生代理池采购需求；指定单节点、严格固定及仅静态成员的组也不通过购买无关新 ID 来修复。

来源补池需求遵循组选定的供应商、来源标签和代理协议，并按供应商指纹、路由/分组策略及实际入口隔离；停止入口后不继续沿用其需求。显式持续维护模式与手动提取仍按各自配置运行，预算不因路由修改重置，已受理任务不伪取消。详见 [补池说明](replenishment-usage.md)。

短效节点使用 `lease:42` 等实际 ID。新规则或新路由版本引用的短效节点必须尚存在、未到期，激活时再次检查；保存后节点消失会拒绝激活，记录 `routing_node_unavailable` 并保留原 head。已有运行规则随节点到期变为不可用，不伪延长租约；不改变路由的其他配置编辑可以保留这种降级状态。长期路由通常应指向分组。

## 域名来源与 Linux 能力

显式代理给出的目标域名属于 `explicit`。IP 对应的 DNS 缓存属于观测推断，默认不参与域名匹配；显式开启 `use_dns_observations` 后，只接受当前单个未过期候选。多个候选返回歧义/未知，不强选其一；外部 DoH/DoT、ECH 和未观察的查询无法据此推断。

进程条件包含 PID、command 或 cgroup ID，但只有实际、已挂载并验证的 Linux 本机进程来源才可启用。当前显式数据面没有这种来源，启用进程条件会拒绝候选预备；停用的模板可保存。不能把 Daemon PID 当作客户端，也不能把网关客户端或远端 HTTP/SOCKS 客户端的进程声明为可见。

规则能力分别返回 `local_process_available`、`kernel_rule_revision`、`kernel_rules_ready` 和 UDP 数据通道状态。目前进程和内核规则数据面如实为不可用，与已实现的 eBPF DNS 捕获分别报告。实际 Linux 进程匹配及内核/用户态原子版本证据继续由 T11/T15 联合验收，本模块尚未以这些占位状态通过完整 Linux 验收。

当前代理节点都是 TCP-only；UDP-only 的非拒绝规则在数据面不支持时拒绝预备。模拟 UDP 匹配不等于已经能转发 UDP。

## CLI 工作流

```sh
hps rule show
hps rule show --revision 4
hps rule save routing.toml --expected-revision 3
hps rule upsert rule.toml --expected-revision 3 --base-revision 4 --index 0
hps rule disable example-via-group --expected-revision 3 --base-revision 5
hps rule enable example-via-group --expected-revision 3 --base-revision 6
hps rule reorder rule-b rule-a --expected-revision 3 --base-revision 7
hps rule delete rule-b --expected-revision 3 --base-revision 8
hps rule simulate context.json --revision 9
hps rule apply 9 --expected-revision 3
```

`routing.toml` 从规则策略根开始，`rule.toml` 从单个规则根开始，不包含 `proxy.routing` 外层。CRUD/启停/排序返回新候选的 ID，不修改运行 head；下一次编辑用 `--base-revision` 继续该候选，期望版本仍是当前运行版本。显式 `apply` 才激活。其他客户端改变 head 后，过期写入返回 409，CLI 退出码 7。

上下文最少包含目标主机和端口：

```json
{
  "destination_host": "example.com",
  "destination_port": 443,
  "transport": "tcp",
  "source_ip": "192.0.2.10",
  "inbound": "browser-http",
  "inbound_protocol": "http"
}
```

模拟不连接目标、不解析新域名、不提取供应商、不推进轮询或创建/刷新会话。运行版本使用实际选择器状态，候选按即将激活的规则与当前非规则设置、健康和租约预览。输出声明规则版本、读取时刻、匹配轨迹、候选排除原因；时间是准入事实评估时刻，不是新发起的健康探测。未涉及节点准入时为 null。外部声明的 DNS/进程上下文明确作为假设，不当成实际观测。

诊断最多并行 4 项，候选准备和匹配在有界工作持有的许可内执行，客户端离开不会提前释放仍在运行的准备工作；超额返回 429。已保存候选仍持久可查询，不依赖 CLI 或桌面窗口存活。

## API 与共享页面

| 方法 | 路径 | 内容 |
| --- | --- | --- |
| GET | `/api/v1/rules?revision=...` | 运行/期望/查看版本、父版本、规则、差异及能力 |
| POST | `/api/v1/rules/save` | `{configuration,expected_revision,base_revision}` 保存候选 |
| POST | `/api/v1/rules/upsert` | `{rule,index,expected_revision,base_revision}` |
| POST | `/api/v1/rules/delete` | `{id,expected_revision,base_revision}` |
| POST | `/api/v1/rules/toggle` | `{id,enabled,expected_revision,base_revision}` |
| POST | `/api/v1/rules/reorder` | `{ids,expected_revision,base_revision}`，必须完整且无重复 |
| POST | `/api/v1/rules/simulate` | `{revision,context}` |
| POST | `/api/v1/rules/apply` | `{revision,expected_revision}` 显式激活 |

规则页只激活规则变更。候选混有其他设置或基于旧配置时，页面要求基于当前版本另存规则候选；新候选保留当前其他设置。完整配置混合激活应在配置工作台审查，不从规则页静默覆盖其他字段。

`/console/rules.html` 提供独立 MPA：候选/运行切换、JSON 编辑、启停/删除、每页 50 条、上下移动及聚焦卡片时 Alt+↑/↓ 排序、变更审查、明确激活、上下文模拟和有时效的候选解释。未保存编辑离页会提示。Browser 与 Desktop 使用同一页面及接口，无项目登录、账号、权限或代理使用者身份规则。
