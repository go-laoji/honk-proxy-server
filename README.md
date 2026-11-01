# honk-proxy-server

Rust 代理池服务，提供独立 CLI、无桌面 Daemon 和共享 React/Tailwind 多页面控制台。当前已开始工程基础开发，**尚未提供可用的代理数据面**。

## 开发与任务

- [开发任务和进度](docs/development-task-checklist.md)：21 个开发 Issue，覆盖 99 项功能。
- [技术规划](docs/technical-architecture-plan.md)、[控制台功能](docs/console-feature-checklist.md)。
- 全部可达使用者可直接操作已实现功能，无登录、账号或权限验证。
- 代理入口、短效供应商 API 和真实 Linux eBPF 分阶段交付；管理服务启动不代表代理已就绪。

## 无桌面运行

需要固定的 Rust 工具链，见 `rust-toolchain.toml`。CLI/Daemon 不依赖 Tauri 或图形会话。

```sh
cargo build --locked -p hps-cli -p hps-daemon
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

`/healthz` 报告进程存活，`/readyz` 报告代理是否可接入；未初始化代理引擎时返回 503，`status.proxy_ready` 为 false。eBPF 的 compiled/configured/attached/ready 分开报告，未集成时不会显示已经接管。

## 验证

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Linux 数据面需要实际内核环境与上游对象验证；macOS 的管理 API 测试或容器预检不能代替真实 eBPF 验收。
