# 短效供应商、批次和租约

对应 T06。来源配置保存在配置版本中，提取结果保存在独立 SQLite 批次/租约表中。一次提取不会制造新的静态配置版本。CLI、浏览器与 Desktop 使用同一服务，不要求项目账号。

## 配置来源

可在控制台“上游供应商”使用模板，也可通过 CLI 载入单个来源的 TOML：

```sh
hps config workspace
hps provider save configs/providers/yun-free.example.toml --expected-revision 1
hps provider list
hps provider show yun-free
```

示例默认停用。填写并启用后显式保存，保存会创建并激活配置版本。所有增改、启停和删除携带 expected_revision；冲突先重新读取，不能覆盖其他页面的修改。管理配置正等待重启时，先到配置工作台处理。删除停止新准入并保留历史，已有连接按原配置结束；配置回滚可重新恢复匹配且尚未过期的来源。

字段包括 id/name/enabled、kind、endpoint、method（GET/POST）、params、headers、auth、protocol、count、count_parameter、timeout_ms、max_response_bytes、fetch_proxy、upstream_credentials、有效期和快照策略。endpoint 不接受用户信息、查询参数或 fragment，查询字段放入 params。请求头、鉴权、抓取代理凭据、上游凭据和常见密钥参数在接口、导出与 Debug 中脱敏；`[REDACTED]` 保留同 ID 已有值。抓取可直连或显式 HTTP/HTTPS/SOCKS5 代理，默认不继承环境代理、不跟随重定向、不自动重试。

通用 JSON 默认读取 `/data/proxies`，每项为 `{"ip":"192.0.2.1","port":8080,"ttl":300}`。`json.list/host/port/ttl/expires_at` 为 JSON Pointer。TTL 必须为秒；绝对时间默认 Unix 秒，显式 `expiry_milliseconds=true` 才使用毫秒。两者同时存在取更早到期；没有期限信息则必须配置保守 ttl_secs。文本来源每行一个 `host:port` 或 `[IPv6]:port`，必须配置 TTL。

## 快代理与云代理

[快代理示例](../configs/providers/kuaidaili.example.toml) 使用 [getdps 官方契约](https://www.kuaidaili.com/doc/api/getdps/)；签名按 [官方 HMAC-SHA1/token 规则](https://www.kuaidaili.com/doc/api/auth/) 实现。HMAC 先对原参数值排序并签名，再按 RFC 3986 编码一次。订单 API 认证与代理节点的用户名密码认证是两组配置；不把供应商密码交给浏览器。

`query_valid_time=true` 在提取后调用 [剩余时长接口](https://www.kuaidaili.com/doc/api/getdpsvalidtime/)，失败时不会将未知期限节点放入池。部分订单不支持此接口，需要显式配置订单保证的保守 ttl_secs，并将 query_valid_time 设为 false。TTL 从请求开始计算，读取响应和重启不会续期；相同 IP 的新提取也是独立租约。接口的 HTTP/HTTPS 目标能力通过 HTTP CONNECT 使用；快代理此适配器的出站协议选择 http 或 socks5，不把支持 HTTPS 目标当成 TLS 代理端口。

[云代理示例](../configs/providers/yun-free.example.toml) 读取用户指定的 [免费页面](http://www.ip3366.net/free/?stype=1)，支持实际页面的 GBK 字符编码。表格的 HTTP/HTTPS 标签解析为 HTTP/CONNECT 代理。页面没有供应商寿命保证，ttl_secs 仅作为本地缓存退休上限，抓取成功不表示健康可用。已验证公开页面的获取和解析。

### 本地模拟提取和换 IP

[模拟配置](../configs/hps.kuaidaili.mock.toml) 使用假订单密钥、一个代理库存和按需补池。运行以下命令会自动启动本地提取接口、剩余时长接口、故障代理、替代代理和目标服务，并使用临时存储验收：

```sh
HPS_FORBID_HOST_NETWORK_CHANGES=1 cargo test -p hps-application --test kuaidaili_failover
```

测试覆盖 `proxy_list` 入池、代理故障后撤销租约、补到其他端点并重试原请求，也检查预算耗尽、目标站错误透传和 POST 不重放。模拟的 HTTP 提取接口仅允许直接访问数字回环地址；实际供应商仍使用 HTTPS。本地模拟不验证付费订单、余额或白名单。

## 提取与结果不确定

```sh
hps provider extract yun-free --request-id manual-20261101-01 --wait
hps provider task 1
hps provider leases yun-free --before 100
hps provider diagnose kuaidaili quota
hps provider diagnose kuaidaili whitelist
```

提取先持久受理再发网络请求，返回任务 ID。相同 request_id 返回同一任务，不同 request_id 在同来源已有在途任务时合并。CLI 会将请求 ID 写到 stderr；收到响应前连接丢失时使用同一 ID 查询或重试受理。`--wait` 只等待结果，CLI 退出或页面离开不会取消后台提取。已知失败或结果不确定时 CLI 退出码为 5；等待时间用尽仍输出当前任务并提示服务继续运行。

HTTP 库禁用重试和重定向。传输中断、响应丢失、超大或损坏响应、不能确认是否扣费的 HTTP 错误记为 outcome_unknown，保留旧池，不自动再次提取。429 作为明确限流拒绝保存有界 Retry-After，控制器按冷却与预算安排新任务；HTTP 库不重试。重复点击和自动补池不能用新的请求 ID 绕过不确定结果；[补池控制器](replenishment-usage.md) 提供统一预算、限频、退避和显式确认恢复。明确需要再次提取时使用新的请求 ID，并先核对供应商记录。

只读诊断可查询快代理额度和白名单，不会修改白名单、订单或购买。没有接口契约的免费/通用来源返回 supported=false、state=unknown，不能显示虚构余额。

## 池发布、快照与恢复

默认 append_leases 保留旧有效批次；不会因为新批次没有某个 IP 就删除旧租约，也不会按重复端点延长旧租约。动态池按来源和端点去重，保留独立批次记录。replace_snapshot 必须显式设置 full_snapshot=true，minimum_snapshot_percent 默认 50，按已有不同端点数量保护异常缩减。空结果或损坏响应始终保留旧池；主动大幅缩减需要显式调整阈值。

后台结果只有在来源仍启用且提取配置指纹一致时才能发布。来源删除、停用或密钥/接口变化后的旧请求结果标为 discarded。仍有足够寿命的供应商租约发布后即可选用，选点仍遵循路由、分组及节点操作状态。健康探测提供独立的诊断结果，详见 [健康准入](health-usage.md)。

业务流量遇到代理端点连接失败、代理握手失败，或快代理明确返回的端点失效与转发错误时，会立即停止使用失败租约。后台会删除同一来源下该端点的有效租约，提取任务仍可查看，已建立的连接自然结束。启用来源的 `[refill]` 后，如果撤销造成该来源的可用库存归零，控制器会在供应商最小提取间隔、冷却和预算允许时优先提取一次。请求会在原路由下尝试另一个端点。无可用替代时最多等待 5 秒，且不超过 `connect_timeout_ms`。整个请求最多重试一次。已经发给上游的明文 HTTP 请求仅在无请求体的 GET 或 HEAD 时重试。目标站正常返回的 403、429、503 等响应会原样转发。已建立的隧道不会重放。[快代理端点失效与转发错误](https://www.kuaidaili.com/doc/dev/dpshttpresponse/)

内存中使用单调时钟限制期限，重复发布不会延长同一租约。正常退出记录逻辑时钟；正常重启保留未过期租约且期限不变。检测到时钟回拨会退休旧租约；崩溃后无法证明经过时间的旧租约保守撤销，需要重新提取，不推测新的到期时刻。中断的 requesting 任务恢复为 outcome_unknown，尚未发出的 accepted 恢复为 failed。

最多 64 个来源、8 个并行抓取、100000 条活跃租约。运行中每秒清理已到期的供应商租约及其节点操作记录，启动时补清理停机期间到期的数据。撤销但尚未到期的租约最多保留 100000 条。提取任务和新增、拒收、撤销等汇总仍可查询；无租约关联的完成任务保留最近 10000 条，保留窗口内提供 request_id 幂等。来源详情最近 50 个任务，租约每页最多 200 条，控制台每页 50 条。自动水位和预算已由 [T07](replenishment-usage.md) 接入；[健康检测](health-usage.md) 已接入；统一任务和事件由 T14 接续。

## 按端点地区加入代理组

在供应商详情打开「地区入组规则」，点击「新增规则」。填写运营商、省份或城市，也可从已有租约一次填入三个字段。选择目标代理组，或填写新组 ID 并在保存时创建轮询组。每条规则的已填写条件须同时满足，留空表示不限。一个节点可以匹配多条规则并加入多个组。

地区匹配依据是服务商返回的公网端点 IP。在服务端用本地 ip2region 查询。租约表显示端点地区，规则行的目标组链接可查看该供应商在组内的节点。没有可查询的公网端点或缺少所需字段时，该节点不会命中对应规则。常见简称可直接填写，例如「广东」「广州」「中国电信」。

规则热应用于现有已发布租约和后续提取。地区库换版后会重新匹配。目标组的显式成员、整来源、子组等成员条件仍可独立入组，原有标签、协议、健康和有效期限制继续生效。删除组或来源前需要处理引用。
