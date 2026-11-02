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
