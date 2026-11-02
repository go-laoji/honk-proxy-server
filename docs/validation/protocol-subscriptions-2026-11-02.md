# T19 / Issue #24 独立验收记录（2026-11-02）

工作分支 `feature/protocol-subscriptions`，基于 #23 已提交基础。仅本机回环 UDP/TCP、临时 SQLite、受控 HTTP 故障夹具；`HPS_FORBID_HOST_NETWORK_CHANGES=1`，使用本分支独立 Cargo target/dist 和 Playwright 18024。没有触碰宿主 Wi-Fi、DNS、路由、系统代理、原服务或 Desktop bundle，没有运行旧 SystemConfiguration 测试二进制。

## 已执行

| 验收 | 真实结果 |
| --- | --- |
| `cargo test --locked -p hps-config -p hps-provider -p hps-engine -p hps-application -p hps-api -p hps-client -p hps-cli` | 142 项通过；Linux 专属测试在 macOS 上编译条件排除，未计通过 |
| 新增回环 UDP 目标域名/DNS/规则用例后 `cargo test -p hps-engine --test socks_udp` | 5 项通过（前次全量含其中 4 项，新增第 5 项另行通过） |
| `cargo clippy --locked` 上述包 `--all-targets -- -D warnings` | 通过 |
| `npm run typecheck` 与 `npm run build` | 通过，18 HTML 入口、同一 MPA manifest 可被 Daemon 嵌入校验 |
| `playwright test --config playwright.protocol.config.ts` | 独立 18024，1 项综合浏览器验收通过 |
| `scripts/verify-protocol-subscriptions.py --bin-dir <独立target>/debug` | 真实 `hpsd` 子进程 + 独立 `hps` CLI，7 项检查通过；JSON 未含测试 URL token/密码 |

UDP 新增测试实际覆盖 IPv4/IPv6、真正的 SOCKS5 上游 association 链、空 payload、来源 IP/端口、首次有效报文锁端口、FRAG/畸形丢弃、无 UDP 路径 REP=7、TCP EOF 与入口 stop 释放全部 UDP socket/资源、全局/入口目标额度、域名 Business resolver、每目标路由拒绝且无直连回退。旧 UDP egress/connection/health/lease/group 单测和集成测试一同回归，未以 TCP CONNECT 冒充 UDP。

订阅 parser 用例覆盖 URI/base64、百分号凭据、双栈、稳定 ID、Clash HTTP/SOCKS5/TLS 子集、不支持协议/选项、重复/空文档和无敏感错误；Application 用真实 SQLite 重启确认快照保留，真实回环 HTTP GET 返回 Clash、拒绝重定向和坏刷新，验证 CAS、停用和没有 lease/batch/extraction task。typed Rust HTTP client 用例验证 CRUD、刷新、来源查询和节点操作。

嵌套单测验证未知引用、环、深度 8/9、全局展开预算 4096、子组固定/fallback 不被父组越过、UDP 排除；Score 逐项验证证据来源、原始/归一值、权重、贡献、缺证据扣分及总分，不使用价格/地域推断。浏览器真实执行新增来源、导入、刷新重载、来源节点页、父子组 Score 分量、环拒绝、坏刷新保留旧快照。

## 边界与集成

- 无新增 SQLite migration/schema 版本，订阅保存在既有 configuration revisions 内；`subscriptions=[]` serde 默认兼容历史配置。#25 backup 当前 schema 版本不因本变更增加。
- Native 仅增加现有 `api_get/api_post` 白名单和纯路径单测，未新增 OS 写入命令。本分支没有从零编译整套 Tauri；由主代理在合并 #23 后统一 Native 测试/Clippy/打包验收，未声称该项已通过。
- 新 SOCKS5 UDP 入站的 Linux/Windows/macOS 外部客户端或 LAN 验收尚未执行；本次是 macOS 回环真实协议验收。已有 #20 Linux 52 项证据不作为新增入站的通过证据。
- 标准订阅仅验证官方协议/字段契约与自有 HTTP 故障夹具，没有测试付费供应商/真实商业订阅；没有采购要求。短效来源主线和既有免费源证据不被替代。
- 支持协议/Clash 选项、手动刷新、未知真实有效期、主动 UDP 健康未支持等限制详见 [使用说明](../protocol-subscriptions-usage.md)。
- 不 finish、不 push、不合 main、不关闭 #24；按主代理 FIFO 集成复核后处理。

## 合并 #23 后复验

按主代理指示将 `develop ec69bc3` 合并进本分支，得到 `c2e87f0`，自动合并无冲突；保留 #23 alerts、diagnostic/CLI/shared UI 与 Native 路径测试。合并后再执行：subscription Application 3 项、typed API 1 项、UDP ingress 5 项、selection 5 项，合计 14 项通过；`npm run typecheck`、18 页 build 与独立 18024 浏览器综合验收再次通过。中央进度/audit 由主代理维护。

## 解析资源审查修复

主代理审查后修复远端 GET 的解析执行位置：完整有界收包后将 owned 配置/文本送入 `spawn_blocking`，外层 tracked/provider permit 持有到解析完成。额外增加解析前 YAML 锚点/别名拒绝、32 层缩进/flow 深度限制及流式 10000 proxies 上限；引号内 &/* 与注释保持普通内容。serde_yaml_ng 自身虽然有 128 深度及 100×event alias jump 限制，本项目不依赖该较宽扩展上限。focused provider 3 项、Application 3 项及 Clippy 通过，覆盖 alias/深度/10001 条预算与引号内敏感字符串。

## 代际与相邻寿命策略审查修复

只读审查定位原 shared DynamicPool 在新 revision 可见至后置 refresh 之间可能残留旧订阅节点。现将订阅移至每个 candidate Engine 的 `OnceLock` 不可变快照，`prepare_proxy_engine` 在开始接流/发布前完成端点/TLS 预备；共享短效 lease pool 保持原 100000 上限，订阅单独 20000 上限。没有订阅写入后置 lease refresh。订阅/普通组等未修改短效来源的激活跳过无关 lease reload，避免已发布订阅后因该查询失败而误报未应用。

新增确定性回归验证：candidate 未 start/publish/refresh 时新快照已可选、共享 lease Arc 相同但旧代际仍保留旧快照、OnceLock 拒绝重写、TLS 预备故障不改变当前 revision/head。另用真实临时 SQLite `ALTER TABLE` 故障令 lease 查询失败，订阅刷新仍返回 applied 且与持久 head 一致，随后恢复测试表。

相邻审计确认 provider 指纹排除 min_remaining/refill 而旧节点缓存 minimum 存在同类窗口。现由每代际 source lifetime policy 在预备时直接取当前 min_remaining 与 retire_before，统一供选择、最终 admission、ProbePlan/会话使用。回归在相同指纹/共享 lease pool、尚未刷新池时验证旧代际阈值 0 可选，新代际阈值 300 立即拒绝且实际 dial 返回 NoUpstream。

嵌套组的成员/来源/fallback 与节点/来源标签预编译为 HashSet，组引用预编译索引，避免合法大成员表对每个候选做线性成员查找。上述 generation unit、SQLite 故障 Application 4 项、寿命 admission unit、selection 5 项、UDP ingress 5 项、typed API 1 项已通过；最终 Clippy/fmt 检查通过。

代际隔离与寿命修复后，独立 18024 浏览器综合验收再次通过（真实 Daemon，1 项）；`cargo fmt --all --check` 与 `git diff --check` 通过。显式 SOCKS5 UDP 的用户态能力也与 Linux transparent UDP capture 开关分开，关闭透明 UDP 不会关闭已有 SOCKS5 UDP 入站实现。
