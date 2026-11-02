# T06 短效供应商验收

- 日期：2026-11-01（Asia/Shanghai）。对应 [Issue #11](https://github.com/go-laoji/honk-proxy-server/issues/11)。
- 用户提供快代理官方文档并明确没有采购，可按文档实现；云代理免费源单独实测。没有把本地测试服务标为付费供应商真实验收。

## 实现与证据

来源配置通过配置版本增改/启停/删除。Rust 适配器包含快代理 token/HMAC-SHA1、JSON/Text 提取和云代理 HTML/GBK 解析，支持超时、响应上限、抓取出口与上游认证。数据库 schema 3 存储受理任务/批次/独立租约；schema 4 区分正常退出与无法证明经过时间的崩溃恢复。动态池原子发布，来源配置变化前的在途结果不越过指纹发布。

新增测试覆盖官方签名固定向量、编码一次、秒/毫秒与保守起算、JSON/文本/GBK/IPv6、错误脱敏、响应丢失只发一次、重定向拒绝、连续批次、独立租约、重复触发与 ID 幂等、空池/快照缩减保护、重复端点的快照基数、租约分页、重启中断、时钟回拨和正常/异常退出。真实代理测试确认提取到的带认证上游无需重启即可访问目标，停用后禁止新准入；关闭服务不会等待供应商超时。

浏览器 **8 项 E2E**通过。新增流程验证两个真实页面修改冲突、MPA 文档导航、提取中离开与恢复、在途触发合并、真实 HTTP 响应入库、脱敏、保留凭据再保存、租约展示、停用和 390px 布局。截图由实际页面生成并检查，无水平页面溢出；租约表自身可横向滚动。

实际 Desktop `.app` 经原生桥保存启用的云代理来源为 r2，点击提取返回任务 #1，随后窗口显示 **15 条租约入库**、时间和本地缓存期限依据。CLI 查询同一任务和 15 条记录，复用同一 request_id 返回任务 #1；CLI 保存停用快代理模板 r3，再删除为 r4，未使用付费密钥。结束后用 CLI 回滚 r1，期望/运行版本均恢复为 1。没有更改系统代理。

Rust 抓取示例通过 ProviderClient 实际从用户给定免费页获取并解析 **15 个端点**；临时下载文件另经 Rust 解析得到相同数量。没有拨打这些公开代理或声称它们已健康。原始页面保留在临时目录，未将第三方完整 HTML 纳入仓库。

macOS aarch64 与官方 `rust:1.98.1-alpine` 的实际 Linux aarch64 musl **各 49 项 Rust 测试通过**；Docker 只读源代码、独立缓存、无特权。该验证只覆盖用户态代理和租约，未加载 eBPF。Rust fmt/clippy、TypeScript、生产构建、Prettier、Desktop bridge 单测与打包通过。

## 复现

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
pnpm typecheck
pnpm build
pnpm format:check
pnpm test:e2e
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
pnpm tauri build --debug --bundles app
# 只读取免费页面，不拨打其代理
cargo run --locked -p hps-provider --example fetch_free_source
```

边界：付费 getdps、剩余时长、额度及白名单真实账号调用未验证；用户允许按文档交付。T07 自动补池预算、T08 健康、T14 统一任务事件和 T03/T15 真实 eBPF 继续开发，未用本项替代。
