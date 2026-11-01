# Rust 与共享控制台基础验收

- 日期：2026-11-01（Asia/Shanghai）。
- 对应任务：T01 [#6](https://github.com/go-laoji/honk-proxy-server/issues/6)、T02 [#7](https://github.com/go-laoji/honk-proxy-server/issues/7)。
- 环境：macOS aarch64，Rust 1.98.1、Node 24.15.0、pnpm 12.10.1。

## 已验证的结果

1. CLI 和 Daemon 可在没有 DISPLAY/桌面会话的进程环境下运行；独立依赖树不包含 Tauri/GTK/WebKit。
2. 实际启动 `hpsd`，通过 `hps --output json` 查询 status/version/capabilities；无需账号、cookie 或管理令牌。
3. TOML 加载、未知字段拒绝、地址校验、CLI init 不覆盖已有配置、目标 URL 子路径保留已验证。
4. 管理接口正常时 `management_ready=true`；代理尚未实现时 `proxy_ready=false`、`/readyz` 返回 503，eBPF compiled/attached/ready 均为 false。
5. Vite 生成 18 个真实 HTML；生产 Daemon 逐页返回对应文档，未知 HTML 和登录地址返回 404。
6. 浏览器操作证明页面导航创建新文档；刷新、前进后退、移动导航、主题跨页保持与空 cookie 可用通过。
7. 每个嵌入静态文件的 SHA-256 与共享发行清单逐一匹配；API 返回相同 web_build_id。
8. Tauri 原生 bridge 单测与 clippy 通过，生成 macOS debug `.app`（约 40 MiB）。实际窗口中连接本机 Daemon、整页切换概览/供应商/设置、读取真实配置和构建标识通过。

## 可复现检查

```sh
pnpm build
pnpm typecheck
pnpm format:check
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
pnpm test:e2e
pnpm tauri build --debug --bundles app
```

本次共有 3 项 Rust 核心测试、1 项 Desktop bridge 测试、4 项浏览器端到端测试通过。原生窗口使用打包后的本地资产，目标为实际运行的 `127.0.0.1:17900` Daemon。

## Linux 前期观测与未交付范围

Docker Linux 预检读取到内核 `7.0.14-linuxkit`、aarch64、可读 BTF、cgroup2。普通测试容器未挂载 bpffs，能力掩码也不代表已经可加载本项目的 eBPF 程序；本次没有加载 BPF 程序或验证真实透明出口。

HTTP/HTTPS/SOCKS5 代理转发、短效供应商、持久化配置写入、完整业务页面、系统服务管理和真实 eBPF 仍由后续任务交付。控制台未实现模块显示不可用，不提供虚构的节点、健康或转发成功数据。

本次结果只完成工程基础，不代表 M0 或 Linux 正式功能版本已经完成。
