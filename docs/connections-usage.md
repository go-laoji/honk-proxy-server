# 业务连接与流量观测

对应 [T13 / Issue #18](https://github.com/go-laoji/honk-proxy-server/issues/18)。浏览器与 Desktop 共用 `connections.html`，节点详情可以进入该节点的实际连接列表，连接详情可以返回节点与规则版本。Daemon 和 CLI 独立运行，不需要启动桌面，也没有项目登录或操作权限要求。

## 记录和统计边界

每个 HTTP 正向业务交换、CONNECT 或 SOCKS5 隧道是一条记录；显式 Direct 使用同一登记表。入口 TCP 套接字与业务交换分别计数，一个 HTTP keep-alive 套接字可以承载多个交换。记录从实际规则判断、拨号开始，包含失败或拒绝；响应体和隧道实际释放后才退出活动计数。热配置沿用登记表，记录保留流量发生时的规则版本、入口、源 IP/端口、目标、实际节点和选点原因。显式代理无法知道远端应用的进程，界面明确显示未知。

字节数来自协商后出口业务流的成功 read/write，包括部分写入和 vectored write 的实际返回值。HTTP 包括转发首部，隧道包括目标 TLS 密文；不包含上游代理协商或 HTTPS 代理的 TLS 握手，不代表目标最终收到的字节数。健康探测、固定 DNS 诊断不混入业务流量。系统不保存业务路径、首部、正文或上游凭据，不解密 HTTPS 目标内容。

累计量属于本次服务观测周期；每次启动生成新的 `epoch`，连接 ID 为该 epoch 加单调序号，热配置不重置。速率使用同一 epoch 的两次实际累计值与服务单调时间差，单位为二进制字节单位每秒，页面标明采样区间；首样本、重启或读取失败后显示采样中。

## 查询、分页与断开

列表可按来源 IP/CIDR、源端口、目标地址/已使用域名、目标端口、入口/协议/传输、来源类型、节点、供应商、分组、规则、状态、异常及开始时间筛选。目标匹配不会临时解析 DNS。`@static` 表示静态来源、`@direct` 表示直连，未分组和默认动作分别用 `@none`、`@default`。页面 URL 保存筛选条件，可以刷新和返回。默认显示活动记录，终态需选择相应状态。

每次列表创建不可变快照，分页复用同一快照和筛选指纹；新连接、字节变化不会挤入原分页。默认每 2 秒重新采样，勾选连接时暂停自动刷新；隐藏页面暂停采样。快照过期、被淘汰或服务实例变化时，页面清空旧选择并重新取快照，不自动重试断开操作。

单条断开先显示具体 ID；批量断开先预览具体 ID，再确认该预览。没有勾选时预览当前筛选快照中的活动记录，范围跨分页；之后新出现的连接不会被加入。预览不依赖列表快照继续存在，但有自身到期时间。确认结果区分“已请求取消”和“已经结束”；真实 IO 退出后活动计数才归零。取消会唤醒挂起读写，HTTP 交换可能关闭复用的入口 keep-alive 套接字，不换节点、不迁移或重放业务。

当前上限：10000 条完整活动记录，512 条最近终态且最多保留 5 分钟；超出完整观测容量的业务仍受原连接资源限制，并显式报告未完整观测数量。最多 4 份列表快照、8 份断开预览，分别有效 30 秒；列表每页最多 200 条，单个断开预览最多 500 条。超出断开范围需缩小筛选或显式选择，不静默截断。容量限制对所有客户端共享。长期历史由 T18 接续。

## API 与 CLI

| 方法与路径 | 用途 |
| --- | --- |
| `GET /api/v1/connections` | 筛选、冻结快照和分页；返回 epoch、快照时刻与容量信息 |
| `GET /api/v1/connections/summary` | 实际业务统计、入口统计与当前运行事项 |
| `GET /api/v1/connections/{id}` | 当前完整记录或最近终态，保留实际路由版本 |
| `POST /api/v1/connections/disconnect` | `{id, expected_epoch}`，请求取消单条连接 |
| `POST /api/v1/connections/disconnect/preview` | `{snapshot, expected_epoch, ids: []}`，空 IDs 表示整个已冻结筛选范围 |
| `POST /api/v1/connections/disconnect/confirm` | `{token, expected_epoch}`，确认具体预览；有效期内重复确认返回同一受理结果 |

分页请求携带 `snapshot`、`expected_epoch`，并保留原筛选；仅 `offset`、`limit` 可以改变。实例变化、快照失效、筛选不一致返回结构化 409，输入错误 422，诊断容量忙 429，记录不可用 404。浏览器和原生桥均保留错误码；桌面只开放固定 API，不接受任意转发 URL。

```sh
hps --endpoint http://127.0.0.1:17890 connection summary
hps --endpoint http://127.0.0.1:17890 connection list --node node-a --limit 50
hps --endpoint http://127.0.0.1:17890 connection list --snapshot SNAPSHOT --expected-epoch EPOCH --node node-a --offset 50 --limit 50
hps --endpoint http://127.0.0.1:17890 connection show CONNECTION_ID
hps --endpoint http://127.0.0.1:17890 connection disconnect CONNECTION_ID --expected-epoch EPOCH --confirm
hps --endpoint http://127.0.0.1:17890 connection disconnect-preview --snapshot SNAPSHOT --expected-epoch EPOCH
hps --endpoint http://127.0.0.1:17890 connection disconnect-confirm --token PREVIEW_TOKEN --expected-epoch EPOCH --confirm
```

可加 `--ids ID1,ID2` 缩小预览。CLI 写操作必须带 `--confirm`；它是明确操作范围，不是项目认证。缺确认或非法 ID 退出码 2，版本/实例/快照冲突退出码 7。JSON 返回的具体 epoch、ID、snapshot、token 必须使用本次读取值。

仪表盘摘要关联当前入口、可转发路径、等待重启、最近失败激活、当前启用来源的最新失败提取，以及实际 DNS 内核挂载状态。它不把预检或普通流量观测当作 Linux 透明接管就绪证据；进程与透明流内核代际仍由 T15 的真实数据面接入。
