# T20 / #25 候选验收记录（2026-11-02）

此记录针对 `feature/backup-desktop-diagnostics`；已合入 #23 的 develop `ec69bc3`，Native 直接使用真实 `hps_domain::AlertFeed` / `HpsClient::alerts`。#24 与全部平台组合验收由根代理按依赖顺序集成；当前功能候选不等于所有平台实际 OS 行为已验收。

## 已执行并通过

| 检查 | 结果与证据范围 |
| --- | --- |
| `cargo test -p hps-api -p hps-storage backup -- --nocapture` | 6 通过，0 ignored。Storage 4、API 2。 |
| WAL 活动写入 | 真实 2MiB SQLite/WAL 与持续事务写线程；固定只读事务快照、分页备份期间写入计数继续增长，快照 quick_check/外键/业务字段一致。 |
| 恢复失败与版本 | 运行锁、坏摘要、损坏文件、新 schema、revision 内容摘要错误、缺草稿/head/clock/event_context、未知草稿字段、缺 schema 对象均拒绝；目标主库 bytes 不变。v1 快照保持 v1 原版本恢复，正常 Store 重启再迁移至 v8。 |
| 响应预算 | 第一未消费 Body 保留许可，第二响应 429；drop 第一后第三响应 200。分块独立 64KiB 输出。 |
| 真实 HTTP + typed client | 本地实际 Axum/TCP 下载，metadata/size/digest 检查，恢复锁拒绝，停止 Supervisor 后可恢复。 |
| 真实 CLI + 独立 Daemon | `scripts/validate-backup-offline.py`：随机专属端口、唯一实例名校验、临时数据目录；下载与摘要、运行时恢复拒绝、停止后恢复与原库保留、错误摘要拒绝、重启通过。进程与临时文件均清理。详见 `sqlite-backup-2026-11-02.json`。 |
| 拓扑解析/增量 | proc IPv4 little-endian 与 IPv6 prefix 测试通过；domain 同 epoch/同对象/同 revision/单调计数增量与重置测试通过。 |
| 真实 Linux 只读采集 | 生产 collector 在新建 Docker `--network none --read-only --cap-drop ALL` 环境运行；7.0.14-linuxkit，10 接口 / 3 路由 / 1 cgroup 挂载，连续采样 namespace 稳定。详见 `linux-topology-2026-11-02.json`；不是完整 Linux API 或 eBPF 数据面验收。 |
| Native 完整测试 | 合并真实 AlertFeed DTO 后 `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib`：11 通过，0 ignored。包含通知基线/epoch/序号去重、私有偏好落盘、Native 旧上下文拒绝、SSE 边界/取消/释放与本机对象守卫。没有实际启用 OS 自启或通知。 |
| Clippy | Desktop `--all-targets -- -D warnings` 通过；backend storage/api/client/cli/platform/domain 六个变更包 `--all-targets -- -D warnings` 通过。 |
| TS / shared MPA | 两份 TypeScript 配置 typecheck、18 份共享 MPA Vite build、改动文件 Prettier 与 Rust fmt/diff 检查通过。 |
| 浏览器真实 E2E | 专属 17993、持久临时测试库：真实 SQLite 下载与摘要/凭据确认、Linux 诊断读/刷新、已有 Linux 预检，3 通过；390px 无横向溢出。没有 mock 下载或 mock backend。 |
| macOS 独立包 | 仅本地 debug `.app`，identifier `io.bitdata.hps.issue25.acceptance`，产品名 `HPS Issue25 Acceptance`，使用独立 `desktop-target`；adhoc 签名及 codesign 校验通过。没有覆盖 `src-tauri/target` 原 bundle，没有启动 GUI。 |

## 尚未验收与边界

- 两次独立的只读 CUA 库存均报告 Mac locked；未自动解锁、未要求用户反复解锁，也未启动候选。原生窗口、托盘点击/关闭到托盘、实际系统通知展示与真实登录自启尚未实机验收。编译、单元测试、adhoc 包只证明对应静态/无 OS 副作用路径。
- Linux 生产 collector 已在真实容器内只读运行；带已加载程序/maps 的完整远端 API/CLI/MPA 与最终 #24 合并后的真实内核组合验收交由根代理统一执行。现有 ABI4 对象与 52 项 6.12.115/7.2.14 数据面报告不由此项重新宣称为新通过。
- 通知复用 Daemon feed，无服务端 ack；游标先保留再排入 OS，崩溃/OS 拒绝可能漏展示，不保证 exactly-once delivery。系统通知开关默认关闭，OS 勿扰可能影响展示。
- 恢复为显式离线维护，不保证无缝热恢复；外部证书/文件、Desktop 偏好和 OS 网络设置不在 SQLite 快照中。
- 全程未写宿主 Mac Wi-Fi/DNS/路由/代理配置，未运行旧 ignored SystemConfiguration 测试或旧二进制，未启停用户 17900 服务，未修改原 bundle、偏好或全局登录项。运行测试/CLI/Daemon 均设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`。

## 可复现入口

```sh
cargo test --locked -p hps-api -p hps-storage backup -- --nocapture
cargo test --locked -p hps-domain topology::tests
cargo test --locked -p hps-platform topology::tests
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings
cargo clippy --locked -p hps-storage -p hps-api -p hps-client -p hps-cli -p hps-platform -p hps-domain --all-targets -- -D warnings
python3 scripts/validate-backup-offline.py --hps ./target/debug/hps --hpsd ./target/debug/hpsd
python3 scripts/validate-linux-topology.py --image hps-rust-arm64:1.98.1
```

Docker collector 脚本只使用已安装镜像（不主动拉取），新建容器结束即删除；它不会使用共享可写 target volume 或原运行服务。默认 Playwright 的 ephemeral server 也可执行新增用例，会验证备份明确拒绝非持久存储；完整下载证明来自本次专属持久测试 server。
