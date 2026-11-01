# honk-proxy-server

自主实现的 Rust 代理池服务，提供独立 CLI、无桌面 Daemon 和共享 React/Tailwind 多页面控制台。当前已开始工程基础开发，**尚未提供可用的代理数据面**。

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
```

默认管理地址为 `127.0.0.1:17900`，通过 `hpsd --listen` 或 TOML 调整；CLI 使用 `--endpoint` 或 `HPS_ENDPOINT` 选择目标。`hps --version` 查看本地 CLI 版本，`hps version` 查询目标服务版本。`hps ui` 仅输出控制台 URL，只有显式 `--open` 才打开浏览器。

## 共享 Web 与 Desktop

前端在 `web/console`，包含 18 个实际 HTML 入口。原始未使用的演示素材未纳入版本管理。默认入口、运行概览、启动设置与 eBPF 状态读取真实 Daemon API；尚未实现的业务模块明确显示不可用，不使用演示节点填充。

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

`/healthz` 报告进程存活，`/readyz` 报告代理是否可接入；未初始化代理引擎时返回 503，`status.proxy_ready` 为 false。eBPF 的 compiled/configured/attached/ready 分开报告，未集成时不会显示已经接管。

## 验证

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm test:e2e
```

首次运行浏览器测试需要 `pnpm exec playwright install chromium`。测试会启动真实 Daemon，验证多页面导航、无账号访问、共享资源哈希、移动布局和状态准确性。`cargo test --workspace` 包含 Web 资源 crate，先执行 `pnpm build`；仅构建 CLI/Daemon 精简包则没有此要求。

Linux 数据面需要实际内核环境与上游对象验证；macOS 的管理 API 测试或容器预检不能代替真实 eBPF 验收。
