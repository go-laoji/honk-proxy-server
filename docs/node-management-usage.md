# 节点库存与操作

`nodes.html`、`hps node` 与 `/api/v1/nodes` 使用同一运行状态。页面可以直接刷新，名称/端点、来源、分组、协议、能力、状态、有效期、排序与分页保存在 URL。分页显示实时库存，期间发生提取或到期时结果可能变化；所有写操作仍校验所见配置与操作版本。

## 库存、健康和寿命

静态节点来自版本化的 `proxy.upstreams`。供应商节点保留独立的 `lease:<id>`、来源指纹、批次、接收时间、原始到期时间和有效期依据。历史查询最多保留 200000 条租约的有界投影，优先返回仍有效的库存；达到上限时明确标注摘要截断。正常历史清理由供应商存储既有的 30 天/100000 条退休记录上限控制。

库存、进入当前池、可准入分别统计。去重、水位、来源停用/修改可能使一个有效租约没有进入当前池。节点自身可准入不代表满足任意入口、分组或规则；实际选点仍由统一选择器判断。健康按检测配置、TCP/UDP、IPv4/IPv6 和证据时间区分，不把已导入、端口可连或租约到期当成穿透健康结果。当前 UDP 返回不支持；IPv4/IPv6 能力筛选要求真实的新鲜健康证据。显式关闭健康检查时可准入不代表已经检测通过。

详情展示来源、批次、组、标签、寿命、凭据是否配置、最近健康证据和实际拨号/连接数。凭据不出现在列表、详情或预览中。计数包括已准入的在途拨号和持有套接字，探测不计入；T13 在同一登记表上补充逐连接记录、流量与主动断开。HTTPS 正文始终保持端到端加密。

## 人工操作与排空

启用操作是解除人工限制，配置禁用、来源失效、健康不足和寿命限制仍然生效。禁用、排空与软删除都立即阻止新的业务拨号；此前已准入的拨号及套接字自然排空，不迁移或重放业务数据。排空归零后保持排空状态，直到解除限制。新增自动健康调度跳过受限节点，已排队或在途探测不会被伪装为已取消。

软删除可以通过解除限制恢复。供应商节点操作必须包含所见租约 ID、批次与来源，仅抑制该租约，不删除整批、其他同端点租约或来源，也不续期、不回滚提取费用。同端点去重优先保留未受人工限制的租约。动作本身不调用供应商提取接口；已显式配置的补池策略仍按实际需求、水位与预算运行。静态配置的物理删除通过配置版本工作区完成，保留规则与分组引用校验。

操作状态通过 SQLite 单事务保存，具有独立的单调版本；同一状态再次写入也递增版本。请求同时携带运行配置版本，409 表示需要重新读取事实后再操作。状态跨热更新和重启保留；旧引擎与当前引擎共用准入门和计数。稳定 ID 表示同一个受控节点，修改静态 ID 表示新节点。

响应 `pool_refreshed=false` 表示操作门已生效，但运行池重算未完成；刷新后可以按新操作版本重试。客户端离开不会取消已受理的写操作；服务关闭等待所拥有的任务结束。

## 手工添加与导入

逐行地址支持 HTTP、HTTPS、SOCKS5、SOCKS5h，及使用指定默认协议的 `host:port`；IPv6 使用 `[地址]:端口`。HTTP/HTTPS 省略端口时使用 80/443，SOCKS5 使用 1080。URL 路径必须为空或 `/`，不接受查询参数和 fragment；凭据中的特殊字符应做 URL 百分号编码。`#` 开头的整行注释和空行忽略，预览行号保持原始文件行号。

JSON 格式是严格的 `UpstreamConfig` 数组，可指定静态 ID、到期时间和 TLS 配置；未知字段拒绝。单次最多 1 MiB、1000 个条目，逐行文件最多 2000 行。逐行格式使用规范化节点内容生成稳定 ID，同一内容重新导入会显示重复 ID；导入只追加，不隐式覆盖已有节点或凭据。

先预览再确认应用。无效、不支持和重复条目分别标注；默认任一无效行都会阻止整批应用。可以明确选择有效行号后重新预览，应用仅限所选行。预览不解析 DNS、不拨号、不探测、不购买；文件、格式、协议、所选行和配置版本共同生成校验摘要。修改输入或选择后必须重新预览。最终激活仍需通过资源、TLS 与配置版本校验，失败保留原运行配置。

## CLI 示例

```sh
hps node list --provider @static --protocol http --state eligible --limit 50
hps node list --provider vendor-a --sort expiry_asc --remaining-max-secs 60
hps node show lease:42
hps node drain lease:42 --expected-revision 7 --expected-control-version 0
hps node delete lease:42 --expected-revision 7 --expected-control-version 1
hps node enable lease:42 --expected-revision 7 --expected-control-version 2
hps node import-preview ./nodes.txt --expected-revision 7
hps node import-preview ./nodes.txt --rows 1,3 --expected-revision 7
hps node import-apply ./nodes.txt --rows 1,3 --expected-revision 7 --checksum PREVIEW_CHECKSUM
```

`@static` 专门表示静态节点；名为 `static` 的供应商仍可通过 `--provider static` 独立过滤。CLI 从详情读取不可变的租约归属，但继续使用命令行提供的配置和操作版本进行 CAS。冲突退出码为 7。可使用 `--endpoint` 指定独立 Daemon，`--output json` 获取结构化结果；无需启动桌面程序。

## API

```text
GET  /api/v1/nodes
GET  /api/v1/nodes/{id}
POST /api/v1/nodes/action
POST /api/v1/nodes/import/preview
POST /api/v1/nodes/import/apply
GET  /api/v1/health?node=lease%3A42
```

查询支持 `id/search/provider/group/protocol/capability/state/remaining_max_secs/sort/offset/limit`，每页 1–200 条，偏移最多 300000。查询和写入共用每实例最多 4 个节点操作槽位，计算在受限的阻塞工作线程完成，取消客户端请求不会提前释放未完成工作的槽位。所有可访问服务的用户都可直接使用这些接口，不要求项目登录或权限。
