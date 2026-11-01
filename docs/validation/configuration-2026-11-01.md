# T05 配置版本验收

- 日期：2026-11-01（Asia/Shanghai）。
- 对应 [Issue #10](https://github.com/go-laoji/honk-proxy-server/issues/10)，覆盖 CFG-01–06、SYS-01、SYS-03。

## 交付和验证

单写者 SQLite、可逆 schema 1/2 迁移、共享草稿 CAS、不可变版本、激活历史、已应用版本回滚、数据目录独占锁和有界保留均已实现。CLI、API 与共享 React/Tailwind MPA 使用相同工作流，无项目登录与账号表。

macOS aarch64 工作区 **33 项 Rust 测试**通过，包含 8 项应用层测试、3 项存储测试、3 项 API 测试，以及已有 12 项真实代理测试。覆盖：持久化重启、未知较新 schema 拒绝、上下迁移、并发草稿冲突、校验/端口/TLS 准备失败保留旧代际、同监听热切换、旧连接跨激活持续、跨代际连接上限及停止、需重启字段、已应用版本回滚、脱敏与草稿刷新后的新凭据/轮换凭据引用。

SQLite 故障注入中，外部写锁阻塞激活后丢弃客户端调用，后台仍完成一致切换。该测试发现 WAL 延迟事务升级的问题，写事务已改为 IMMEDIATE。保留期测试确认活跃代际与草稿基线不会被误删，外键检查通过。

官方 `rust:1.98.1-alpine` 镜像中在实际 Linux aarch64 musl 上编译运行同一组 **33 项测试**通过。源代码隔离只读挂载，使用独立 Cargo/target 缓存，没有特权网络或 eBPF；该结果仅验证配置与显式代理。镜像 digest：`sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f`。

**7 项浏览器 E2E**通过，其中新增配置流程使用两个实际页面验证 409 冲突、草稿刷新、保存/激活、实际实例状态、脱敏下载、pending_restart、回滚和 390px 布局。页面保留真实 HTML 入口，未引入 SPA 回退或账号 Cookie。

独立 CLI/Daemon 实际进程验证配置导入/校验/激活/重启/回滚和冲突退出码 7；新数据库文件为 0600。管理 HTTPS 使用临时 CA 验证不可信证书拒绝、显式 CA 成功，以及慢握手不阻塞其他客户端。另以占用管理端口重现无法重启，再用 `--recover-revision 1` 离线恢复成功。

Desktop `.app` 打包、桥 allowlist 单测和 clippy 通过。实际窗口在 `tauri://localhost/config.html` 经原生 POST 桥保存草稿 v2、校验、保存 r2 并激活，实际实例名变为 `desktop-config-check`。原生 Save 面板导出 `/tmp/hps-desktop-config-check-20261101.toml`，文件包含格式头与 r2 内容；随后窗口回滚 r1，CLI 确认期望与实际运行版本均为 1。未更改系统代理设置。

工作区 fmt/clippy、前端生产构建、包含 E2E 的 TypeScript 类型检查和 Prettier 检查通过。Web build ID 为 `e245c04886296af9b803f135e632c15ec02b300c95456e0a9584cb440892460e`；实际运行值以 `/api/v1/status` 为准。

## 复现与边界

```sh
cargo fmt --all --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
pnpm build
pnpm typecheck
pnpm format:check
pnpm test:e2e
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
pnpm tauri build --debug --bundles app
```

使用说明见 [配置版本](../configuration-usage.md)。管理 TLS、管理监听和日志过滤需重启，未声称热替换管理服务。统一任务/SSE 和活动保留天数在 T14 接入；Desktop 自定义 CA 档案在 T16 完成。本项不代表短效供应商或真实 Linux eBPF 已完成。
