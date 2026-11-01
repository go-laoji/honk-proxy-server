# honk-proxy-server

自主实现的 Rust 代理池服务，提供独立 CLI、无桌面 Daemon 和共享 React/Tailwind 多页面控制台。当前已交付 HTTP/CONNECT、HTTPS 代理与 SOCKS5 TCP、配置版本、短效供应商提取、批次租约、预算补池和统一健康检测；分组路由和真实 Linux eBPF 仍在开发。

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

[健康检测](docs/health-usage.md) 默认启用，按 TCP/UDP、IPv4/IPv6 分开记录端点、认证和目标穿透证据；统一控制新选点、寿命与预算。支持单个/批量后台任务、真实流量分层反馈、CLI/API 与共用控制台。UDP 当前如实报告不支持，重启后重新取得成功证据。

前端在 `web/console`，包含 18 个实际 HTML 入口。原始未使用的演示素材未纳入版本管理。连接、运行概览、代理入口、启动设置与 eBPF 状态读取真实 Daemon API；尚未实现的业务模块明确显示不可用，不使用演示节点填充。

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

Linux 数据面需要在实际内核环境验证本项目自研 eBPF 程序及真实出口；macOS 的管理 API 测试或容器预检不能代替真实 eBPF 验收。当前预检证据见 [Linux 预检验收](docs/validation/linux-preflight-2026-11-01.md)。
