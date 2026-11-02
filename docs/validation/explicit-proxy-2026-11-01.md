# T04 显式代理验收

- 日期：2026-11-01（Asia/Shanghai）。
- 任务：[T04 / Issue #9](https://github.com/go-laoji/honk-proxy-server/issues/9)，覆盖 IN-01、IN-03、IN-04、IN-05、IN-06、SYS-02。
- 后端全部 Rust，自主代理核心位于 `crates/engine`，未引入 honk crate、二进制、对象或 fork。

## 交付结果

HTTP/1.1 正向/CONNECT、TLS HTTP/CONNECT 入口、SOCKS5 无认证 CONNECT 入口共用拨号边界和资源管理。上游支持 HTTP、HTTPS、SOCKS5、供应商认证及静态/到期租约准入。HTTP 正向向 HTTP/HTTPS 上游发送 absolute-form，通过 SOCKS5 时发送 origin-form；隧道保留域名和端到端目标 TLS。

Daemon 实际启动入口、回滚不完整启动并在退出时回收；API、CLI 与共享 MPA 提供实际监听、启停、证书有效期、资源限制、命中计数和接入说明。无本项目登录、账号、权限验证或入站来源 ACL。配置来自启动 TOML，动态启停不写回文件；持久化编辑与发布属于 T05。

## 真实协议矩阵

每个格子都验证了 HTTP 与端到端 HTTPS 两条路径，每条上传并校验回传 2 MiB 数据。HTTPS 入口与 HTTPS 上游均完成真实 TLS 握手，测试仅信任显式提供的临时测试 CA，不跳过证书校验。

| 入站 / 上游 | HTTP | HTTPS | SOCKS5 |
| --- | --- | --- | --- |
| HTTP / CONNECT | 通过 | 通过 | 通过 |
| HTTPS 代理 | 通过 | 通过 | 通过 |
| SOCKS5 TCP | 通过 | 通过 | 通过 |

另外验证：

- 上游 HTTP Basic / SOCKS5 用户密码握手；不可信 CA、错误服务器名拒绝。
- 客户端代理凭据及逐跳头隔离、目标 Host 重建、上游 407 变为网关错误；隧道不检查加密正文。
- 空池、禁用、过期和剩余寿命不足不联系上游、不隐式直连；握手超时 HTTP 504。
- 供应商已收到 POST 业务数据后关闭连接，本项目返回 502，第二个节点未收到重放请求。
- IPv6、TCP 双向半关闭、chunked/trailer 请求、无界大头拒绝、重复冲突 Content-Length 拒绝、越界目标端口拒绝、HTTP Upgrade 返回 501。
- 全局/入口连接上限命中、入口停止/重启、取消、部分启动失败回收、空闲超时、入口限速与并发连接共享全局限速。
- SOCKS5 仅接受无认证方法，BIND / UDP ASSOCIATE 明确不支持。

## 环境与端到端证据

macOS aarch64 工作区共 **20 项 Rust 测试**通过，其中 **12 项代理核心测试**（4 项拨号、8 项运行时）；fmt 和 clippy 通过。无 DISPLAY/WAYLAND_DISPLAY 的实际 Daemon + CLI 查询、启动、停止和零连接回收通过。

同一锁定代码在官方 `rust:1.98.1-alpine` 的 Linux aarch64 musl 环境实际编译运行，**12 项代理核心测试全部通过**。镜像 digest 为 `sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f`，仓库只读挂载，不使用特权参数。

**6 项浏览器 E2E**通过：18 页真实文档/未知页 404、共享资源哈希、多页面导航、移动布局/主题/无账号、真实入口启停跨刷新保持，以及 Chromium 实际代理路径。后者访问不能直接解析的 `origin.invalid`，通过本项目 Daemon 和带凭据的本地供应商协议测试服务到达本地站点；浏览器没有代理账号，站点没有收到供应商代理凭据。

Desktop bridge **1 项测试**和 clippy 通过，macOS debug `.app` 打包成功。实际新窗口加载共享入口页面，通过原生 POST 桥启动 `browser-http`，CLI 查询相同的 `127.0.0.1:50291` 监听；随后从窗口停止，确认未监听、活动连接为 0。测试后恢复停止状态。

前端生产构建、包含浏览器测试代码的类型检查和格式检查通过。浏览器嵌入资产及 Desktop 读取同一 `dist`；本次 Web 构建标识为 `9c5e9332a2e70b2ec919a7f99ef4b4dde43389b225a9f4591ce56e0d706e07ce`。

## 可复现命令

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

docker run --rm \
  --mount "type=bind,source=$PWD,target=/work,readonly" \
  --workdir /work --env CARGO_TARGET_DIR=/tmp/hps-target \
  rust:1.98.1-alpine cargo test --locked -p hps-engine
```

## 验收边界

当前支持启动 TOML 中的静态节点和显式有效期，尚无真实供应商 API 提取、自动补池、健康探测、组/规则发布、SQLite 配置版本或完整连接观测。上述本地供应商服务用于真实协议互通测试，不能代替 T06/T17 的外部短效供应商验收。

本次 Linux 验证的是显式 TCP 代理；没有加载 eBPF 或证明透明 TCP/UDP 出口。T03/T15 仍保持未完成，Linux 正式版本仍须满足真实 eBPF 发布门槛。T04 的完成不代表整个项目或 M0 已完成。
