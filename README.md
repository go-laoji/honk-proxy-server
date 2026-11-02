# honk-proxy-server

自主实现的 Rust 代理池服务，提供独立 CLI、无桌面 Daemon 和共享 React/Tailwind 多页面控制台。当前已交付 HTTP/CONNECT、HTTPS 代理与 SOCKS5 TCP/UDP、配置版本、短效供应商提取、批次租约、预算补池、健康、DNS、分组、规则、连接和任务。Linux 自研 eBPF 支持明确本机/LAN 范围的双栈 TCP/UDP、可信本机进程、原子代际及故障恢复，已在 aarch64/x86_64 的 6.12.115 LTS 与 7.2.14 完成实际矩阵。Desktop/独立系统服务及可验证分包已交付发行候选，平台和实际验收限制见发行文档。

honk 仅作为能力参考；项目不集成其 crate、二进制或 fork。

## 开发与任务

- [开发任务和进度](docs/development-task-checklist.md)：21 个开发 Issue，覆盖 99 项功能。
- [技术规划](docs/technical-architecture-plan.md)、[控制台功能](docs/console-feature-checklist.md)。
- 全部可达使用者可直接操作已实现功能，无登录、账号或权限验证。
- 代理入口、短效供应商 API 和真实 Linux eBPF 分阶段交付；管理服务启动不代表代理已就绪。

## 无桌面运行

需要固定的 Rust 工具链，见 `rust-toolchain.toml`。CLI/Daemon 不依赖 Tauri 或图形会话。

```sh
cargo build --locked -p hps-cli -p hps-daemon --no-default-features
./target/debug/hps init --config hps.toml
./target/debug/hps config check hps.toml
./target/debug/hpsd --config hps.toml
```

另一个终端查询：

```sh
./target/debug/hps --output json status
./target/debug/hps capabilities
./target/debug/hps config show
./target/debug/hps linux preflight
./target/debug/hps inbound list
```

默认管理地址为 `127.0.0.1:17900`，通过 `hpsd --listen` 或 TOML 调整；CLI 使用 `--endpoint` 或 `HPS_ENDPOINT` 选择目标。`hps --version` 查看本地 CLI 版本，`hps version` 查询目标服务版本。`hps ui` 仅输出控制台 URL，只有显式 `--open` 才打开浏览器。

## 使用显式代理

参考 [代理配置示例](configs/hps.proxy.example.toml)，填写实际供应商节点并启用。示例中的上游默认禁用，无合格上游时拒绝转发，不隐式直连。

```sh
./target/debug/hpsd --config configs/hps.proxy.example.toml
./target/debug/hps inbound stop browser-http
./target/debug/hps inbound start browser-http
curl --proxy http://127.0.0.1:17901 https://example.com/
```

上游支持 HTTP/HTTPS/SOCKS5 与供应商凭据，按启用和剩余有效期准入；上游 TLS 默认验证，不提供跳过验证选项。入口启停、证书有效期、有效资源限制与接入示例可在 CLI/API/共享控制台查看。配置工作台支持共享草稿、版本校验、激活、导出和回滚；详见 [配置版本使用说明](docs/configuration-usage.md) 与 [接入及运行语义](docs/explicit-proxy-usage.md)。

Daemon 默认把配置保存在 `.hps/hps.sqlite3`，可通过 `--data-dir` 指定独立目录。TOML 只初始化新目录，已有目录启动时读取数据库期望版本。测试使用 `--ephemeral` 保存在内存中。CLI/Daemon 不需要启动桌面程序。

## 共享 Web 与 Desktop

短效来源支持快代理文档契约、通用 JSON/文本 API 与云代理免费页面，提取结果可直接发布到运行池；提供 CLI 和共享页面。详见 [供应商配置与提取](docs/providers-usage.md)，示例位于 `configs/providers/`，默认停用。快代理未采购，付费真实调用未验证；免费来源抓取成功不等于代理健康。提取请求不自动重放，空结果不清池，重复 IP 不延长旧租约。

[自动补池](docs/replenishment-usage.md) 默认停用，启用后支持按需或显式持续维护、退休预测、水位和统一 UTC 每日预算；手动提取也占预算。不确定结果暂停，确认恢复保留预算与冷却。通用 API 必须按供应商文档配置数量参数，面板区分已健康准入与待检测库存。

[健康检测](docs/health-usage.md) 默认启用，按 TCP/UDP、IPv4/IPv6 分开记录端点、认证和目标穿透证据；统一控制新选点、寿命与预算。支持单个/批量后台任务、真实流量分层反馈、CLI/API 与共用控制台。当前主动 UDP 探测如实报告不支持；这与实际显式/透明 UDP 数据面分别记录，不能虚构健康证据，重启后重新取得成功证据。

[DNS](docs/dns-usage.md) 支持 System/UDP/TCP/DoT/DoH、按用途和域名选路、正负 TTL 缓存、诊断及清理。System 不虚构 TTL；可选静态代理承载 TCP DNS。Linux 自有八个 eBPF hook 已实际验证双栈 TCP/UDP DNS 捕获、对端地址恢复及与业务透明捕获共存；默认不开启监听或捕获。

[节点分组](docs/groups-usage.md) 支持成员/来源/标签过滤、固定与显式回退、健康延迟优选、轮询、加权轮询、受限嵌套组和可解释 Score，三个代理入口共用同一选择器。会话固定受健康和剩余寿命约束；CLI 与共享 MPA 可以管理、预览候选并解释排除原因，模拟不产生网络和补池副作用。

[规则路由](docs/routing-rules-usage.md) 支持首条命中的域名/CIDR/端口/传输/入口/来源组合，执行节点、组、明确直连或拒绝。共享规则页与 CLI 将候选保存和显式激活分开，提供排序、引用校验和无网络副作用的模拟。Linux 本机进程只取自可信内核元数据，实际规则能力和内核代际随捕获状态报告；显式及网关客户端进程保持未知。

[Linux 透明接管](docs/linux-transparent-usage.md) 默认关闭，配置明确 cgroup/LAN 网卡，提供实际接口、link、路由、代际、资源及计数。故障先暂停、最多三次原位修复；独立 hpsd 的 SIGKILL 残留回收、外部冲突拒绝和持久重启均已实际验证。API、CLI 和共用 Linux MPA 使用同一 owner，范围变化走候选审阅/激活并明确等待重启；停止透明流保持显式入口。

[节点管理](docs/node-management-usage.md) 统一查询静态配置、独立订阅快照与供应商租约，提供有界导入预览、软删除恢复、排空、来源水位，以及健康/寿命/人工限制分离；CLI 与共享节点 MPA 操作同一事实源，已有业务连接自然排空。

[任务与结构化事件](docs/tasks-events-usage.md) 提供持久统一任务、按请求 ID 恢复、后台配置提交、关联资源和事件查询。CLI 与共用 MPA 支持有界 SSE、暂停补取、实例变化重置；Desktop 通过 Rust 带确认订阅取消旧目标。离开页面不会取消已受理工作，TCP 对端不代表已认证操作人。

前端在 `web/console`，包含 18 个实际 HTML 入口。原始未使用的演示素材未纳入版本管理。连接、运行概览、代理入口、启动设置与 eBPF 状态读取真实 Daemon API；能力与就绪状态分别呈现，不使用演示节点填充。

```sh
pnpm install --frozen-lockfile
pnpm build
cargo build --locked -p hps-daemon
./target/debug/hpsd
# 浏览器打开 http://127.0.0.1:17900/console/
pnpm tauri dev
```

标准 Daemon 默认通过 `web-ui` 特性嵌入 `dist`；仅用 Rust 构建服务器精简包时使用 `--no-default-features`，无需先构建前端。纯 CLI 构建不依赖 Web 资源。

Vite 生成文件哈希与 `web_build_id`，Daemon 与 Tauri 读取同一份 `dist`。Desktop 使用 Rust transport 调用目标 API，可在连接页设置并保存目标地址；当前不会自动安装或启动代理系统服务。

桌面复用现有 `src-tauri`，以独立 Cargo manifest/锁文件隔离图形依赖。完整安装、托盘、系统代理与持久服务管理由后续 T16 交付。

`/healthz` 报告进程存活；`/readyz` 为 200 代表存在运行入口与可准入的配置上游，HTTPS 入口证书还须在有效期内。默认启用健康时还要求有新鲜可达证据；显式停用健康时会说明未检测准入。未配置、全部停止、节点到期或正在关闭时返回 503。eBPF 的 compiled/configured/attached/ready 分开报告，显式代理运行不代表已经接管 Linux 流量。

`hps linux preflight` 读取目标 Daemon 的默认路径环境信息，经 `/api/v1/linux/preflight` 返回内核、BTF、cgroup2、bpffs、进程能力和可读内核选项。非 Linux 如实返回 unsupported；命令不挂载文件系统、不加载 BPF、不更改网络，也不把环境信息当作透明代理就绪证明。

## 验证

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm test:e2e
```

首次运行浏览器测试需要 `pnpm exec playwright install chromium`。测试会启动真实 Daemon，验证多页面导航、无账号访问、共享资源哈希、移动布局和状态准确性。`cargo test --workspace` 包含 Web 资源 crate，先执行 `pnpm build`；仅构建 CLI/Daemon 精简包则没有此要求。

Linux 数据面需要在实际内核环境验证本项目自研 eBPF 程序及真实出口；macOS 的管理 API 测试或容器预检不能代替真实 eBPF 验收。只读条件和实际挂载分别验收，见 [eBPF 使用与隔离验证](docs/linux-ebpf-usage.md)、[实际内核验收](docs/validation/linux-ebpf-core-2026-11-01.md) 和 [前期只读预检](docs/validation/linux-preflight-2026-11-01.md)。

[历史观测、脱敏诊断与告警](docs/observability-usage.md) 提供当前运行期有界趋势、健康持久历史、规则命中与批量导入导出、只读诊断后台任务和状态变化通知；取消/重试按实际阶段及版本开放，不重放采购。

[标准订阅与 SOCKS5 UDP](docs/protocol-subscriptions-usage.md) 支持 HTTP/HTTPS/SOCKS5 URI、base64 和明确的 Clash 子集，订阅快照按配置代际独立准备，失败保留旧来源，真实有效期未知则保持未知；与短效 API 的租约/预算/补池分开。UDP association 随 TCP 控制连接结束，按来源、目标、规则、健康与容量限制准入。

[SQLite 备份、Desktop 与 Linux 诊断](docs/backup-desktop-diagnostics-usage.md) 提供一致在线备份、严格校验与离线持锁恢复；Desktop 托盘、通知和登录自启选项独立于 Daemon 服务。Linux 只读拓扑与实例已有内核对象/计数共用 API、CLI 和页面，实际验收与平台限制见对应记录。

后续平台与多实例的储备范围见 [产品与兼容契约](docs/future-platform-contracts.md)。这些前置定义不表示未来运行能力已经启用；当前实现与平台验收边界以 [任务清单](docs/development-task-checklist.md) 和对应验收报告为准。
