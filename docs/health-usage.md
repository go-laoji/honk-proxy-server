# 健康检测与准入

HTTP、HTTPS 代理和 SOCKS5 TCP 入口共用一个健康事实源。健康检测默认启用；新静态节点和新提取批次先等待探测，符合寿命要求且有新鲜 TCP 可达证据后才允许新连接。`health.enabled=false` 是显式允许未检测节点的兼容模式，期限、启停和来源配置限制仍生效。

默认 TCP IPv4 探测访问 `https://www.baidu.com/`，TCP IPv6 默认停用。可换成自己的稳定检测站点；HTTPS 验证目标证书、上游 HTTPS 也独立验证证书，支持显式 CA，不提供跳过验证。探测只发送服务自己的 GET，不复制用户业务请求、Cookie 或请求体，不计入按需补池的用户需求。

## 配置

主配置使用 `[health]` 与 `[[health.profiles]]`。CLI 单独保存策略时，文件内容从策略根开始，例如：

```toml
enabled = true
max_parallel = 8
queue_capacity = 256
timeout_ms = 5000
interval_secs = 60
max_staleness_secs = 180
min_probe_margin_secs = 5
daily_probe_budget = 5000
retry_initial_secs = 5
retry_max_secs = 300
traffic_failure_threshold = 2
max_header_bytes = 16384

[[profiles]]
transport = "tcp"
family = "ipv4"
enabled = true
url = "https://www.baidu.com/"

[[profiles]]
transport = "tcp"
family = "ipv6"
enabled = false
url = "https://www.baidu.com/"
```

`max_parallel` 最大 64，队列最多 10000；每天预留次数最大 1000000。超时为 100–60000 ms，周期与最长证据年龄不超过一天，最长年龄不得小于周期。失败按指数退避到 `retry_max_secs`。周期重检期间仍可使用尚未失效的成功证据；过期或失败后不会用旧绿灯补齐可用数量。

每个 TCP/UDP 与 IPv4/IPv6 组合最多一个配置。当前 UDP 数据通道尚未交付，两个 UDP 维度固定报告 `unsupported`，启用 UDP 探测会校验失败。它们不会继承 TCP 成功状态。目标是字面 IP 时，地址族必须与配置一致；域名探测在本地按所需地址族解析，保留目标 Host 与 TLS 名称。该引导解析使用 60 秒成功/5 秒失败的本地缓存，不宣称它是 DNS TTL，也没有替代 T09 的共享 DNS 模块。

URL 不接受账号、查询参数或 fragment。自定义 `ca_certificate` 和 `tls_server_name` 只用于 HTTPS；CA 路径按配置路径规则解析，文件内容参与策略指纹。HTTP 探测目标没有 TLS 身份验证，界面仍分别显示端点、认证、隧道和 HTTP 响应，不能把 HTTP 返回值视为密码学身份保证。

## CLI 与 API

```sh
hps health show
hps health show --limit 50 --after 'lease:100'
hps health probe --node 'lease:12' --request-id inspect-12 --wait
hps health probe --node static-a --node static-b --wait
hps health task 42
hps health save health-policy.toml --expected-revision 3
```

未传 `--node` 时请求当前最多 1000 个节点；队列或预算不足则整个新请求拒绝，不悄悄截短成成功。控制台的批量按钮检测本页或所选节点，分页最多 200 个、默认 50 个。CLI 等待超时后，任务仍由 Daemon 执行，可按 ID 查询；CLI 请求前打印 request_id，受理响应丢失时应复用它。重复受理不新增探测预留，在途的同身份/策略工作也会合并。失败或中断任务的 CLI 退出码为 5，配置版本冲突为 7。

| 方法 | 路径 | 功能 |
| --- | --- | --- |
| GET | `/api/v1/health?after=...&limit=50` | 策略、当前池健康分页、预算及最近任务摘要 |
| POST | `/api/v1/health/probe` | `{request_id,node_ids}`，202 受理后台任务 |
| GET | `/api/v1/health/tasks/{id}` | 完整任务进度及分层结果 |
| POST | `/api/v1/health/save` | `{configuration,expected_revision}`，创建并激活配置版本 |

`health:N` 是健康任务引用；全局任务聚合及 SSE 由 T14 接续。全部接口直接访问，不需要项目账号或管理令牌。Desktop 通过原生桥访问同一路径，浏览器使用同源 API，二者显示相同的任务与配置版本。

## 证据、租约与真实流量

节点身份绑定协议、端点、供应商凭据和实际信任根；探测策略与目标 CA 内容另有指纹。换凭据、换 CA 或换目标后，旧结果不能提升新的身份或策略。相同物理端点的新租约可使用同身份尚未失效的证据，但检测时间不因此刷新，原租约期限也不延长。进程重启后持久结果保留为诊断历史，成功状态降为未知并重新取得证据；中断任务标记 `interrupted`，预算不返还。

探测要求剩余寿命严格大于最小准入寿命、探测余量和整个探测超时之和。自动调度跳过过短租约且不预留次数；用户显式检测过短租约会返回跳过结果，没有节点网络请求，已接受的预留保守保留。过期节点不会因健康证据继续准入。健康新鲜度和租约均以单调截止时间限制，重复发布或亚秒时钟差不会延长准入。

端点失败、上游 TLS、供应商认证、隧道拒绝、目标 TLS/HTTP 和本地问题分别记录。真实端点/上游 TLS/认证失败达到阈值会同步阻止后续选点；本地 socket/内存资源耗尽与检测目标解析失败不会记成节点死亡。403/429 等目标拒绝只作分层反馈，不全局屏蔽节点，也不重放用户请求。真实流量成功不会延长主动探测证据的有效期。

控制台可查看当前进程内的最近流量反馈；它是有界运行态记录，跨进程的完整事件/曲线由 T14/T18 交付。业务隧道内的端到端 TLS/HTTP 无法由代理明文观察，不伪造网站状态或宣称迁移旧连接。

## 与补池协作及资源边界

补池面板分别显示已健康准入、待健康检测和供应商提取在途数量。未知、排队或检测中的有效租约先占库存名额，防止等待检测时重复购买。检测预算耗尽、引导目标不可解析或来源认证/信任异常时，自动购买暂缓；修复配置或检测恢复后重新评估。显式手动提取仍受来源自身预算和暂停规则约束。

每次检测预留在同一个 SQLite 事务内完成，UTC 换日使用持久时钟水位，删除历史与重启不会重置预算。队列按身份/策略去重；工作并发、全部任务数量 5000、运行记录约 100000、端点/策略状态最多 400000 都有上限，状态历史在任务受理时按 30 天上限清理。最近任务列表只返回摘要，详情由任务接口读取。大量重复加入同一个在途探测也不能无限创建活动任务。

本模块的 Linux 用户态测试不等于 eBPF 挂载或透明流量接管；真实 eBPF 仍按 T03/T15 验收。
