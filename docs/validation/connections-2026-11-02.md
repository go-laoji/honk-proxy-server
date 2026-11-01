# T13 业务连接与流量验收

Issue：[#18](https://github.com/go-laoji/honk-proxy-server/issues/18)。核心、API、CLI、共享 MPA 和实际独立原生窗口已通过显式代理验证。依赖 T11 的 Linux 进程/内核代际仍由 T15 联合验收，Issue 保持开放。

## 已验证的实际行为

- Engine 为 Direct、HTTP 交换及隧道登记同一事实源，保留真实源端口、实际路由版本与节点；健康探测不计入。协商后的 IO 对成功读取、部分写入与 vectored write 计数，失败与关闭使用固定类别。10001 个活动观测验证容量溢出显式计数，最近终态上限为 512。
- 真实 HTTP/HTTPS/SOCKS5 上游隧道均验证双向字节、取消和不重放。HTTP 流式体在 1 B/s 反压等待中也能被取消，真实响应体生命周期结束后计数才释放；记录不含业务路径、正文或认证信息。
- Application 的快照、TTL、淘汰、筛选指纹及 epoch 检查可重复验证；断开预览冻结具体 ID。客户端取消诊断读取后，后台工作仍持有操作容量和服务所有权，热配置不重置连接记录。
- 实际 HTTP API typed client 验证旧快照结构化 409、输入 422、单条详情及批量断开；预览后新建流量并淘汰列表快照，确认仅关闭原来预览的连接。
- 独立 CLI/Daemon 子进程创建 4 条真实流，主动断开 3 条；新连接在批量断开后继续回显，每方向精确 23 字节。缺少显式确认退出 2，旧实例 epoch 退出 7。全程没有桌面和供应商购买。
- 浏览器实际创建 3 条经过本地 HTTP 供应商的 CONNECT 隧道。快照淘汰后页面恢复，批量预览包含原 2 条但排除第 3 条；确认后前 2 条真实套接字退出，第 3 条继续传输。单条确认可取消，确认后才真实断开。URL 筛选恢复、跨页范围、节点详情与连接详情双向 MPA 跳转通过，补齐 T12 的连接关联门槛。390px 截图 `/tmp/hps-t13-connections-mobile.png` 已检查，无横向溢出。

## 已运行检查

| 检查 | 实际结果 | 日志 |
| --- | --- | --- |
| macOS aarch64 workspace tests | 140 passed / 0 failed / 0 ignored | `/tmp/hps-t13-release-macos-tests.log` |
| macOS all-target Clippy `-D warnings` | 通过 | `/tmp/hps-t13-release-macos-clippy.log` |
| Linux aarch64 workspace tests + all-target Clippy | 139 passed / 0 failed / 4 ignored，Clippy 通过 | `/tmp/hps-t13-release-linux-tests.log`、`/tmp/hps-t13-release-linux-clippy.log` |
| 全部共享控制台浏览器测试 | 16 passed | `/tmp/hps-t13-browser-all.log` |
| React 与 Node TypeScript 项目 | 通过 | `/tmp/hps-t13-browser-typecheck.log` |
| 完整前端格式检查 | 通过 | `/tmp/hps-t13-release-format.log` |
| 前端生产构建 | 通过 | `/tmp/hps-t13-browser-build.log` |
| 独立 CLI/Daemon 子进程 | 通过 | `/tmp/hps-t13-release-cli-process.log` |
| 最终 Native 桥单测 | 2 passed | `/tmp/hps-t13-release-native-tests.log` |
| 独立 macOS `.app` 打包 | 通过，43.34 MiB | `/tmp/hps-t13-release-native-build.log` |

可复现：`cargo test --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`pnpm typecheck`、`pnpm build`、`pnpm exec playwright test`；构建 headless 二进制后运行 `python3 tests/cli/connections.py`。前端产物更新必须与读取嵌入资源的 Rust 编译分开执行。

Linux 在独立源码快照、Rust 1.98.1 Alpine、单构建任务及独立 Cargo 卷中运行。4 个忽略项是特权 DNS/eBPF 核心/应用及其子进程门槛，本轮不计为执行；此前实际 6.12/7.0 内核证据保持有效，完整透明 TCP/UDP 与进程归属待 T15。

首次新增浏览器夹具将测试凭据写成顶层 upstream 字段，严格配置校验拒绝该请求；改用实际的 `credentials` 表后通过。完整格式检查也发现既有 rules 浏览器测试格式不一致，已单独格式提交并重跑通过。

## 独立桌面实际验收与剩余依赖

`HPS Connection Validation.app`，bundle ID `dev.hps.connection-validation`，独立连接偏好；隔离服务 `127.0.0.1:17999` 无供应商且入口默认关闭。API、共享生产产物与打包应用对应 `web_build_id=169275ac5ccde608668c35ad9b695ced253ca58ed2a8c5093b87e18cd1468837`。原用户 `17900` 服务、原 `.app` 和用户连接偏好保持不动。

2026-11-02 解锁后，通过实际窗口启动入口、从节点详情跳转连接列表，显示真实两条隧道、源端口和每方向 66 B。窗口排空节点只阻止新连接；旧隧道继续传输。候选规则保存不改变流量，激活 r5 后，新请求被拒绝，旧 r4 隧道继续工作。

窗口发现连接 ID 的冒号被前端编码为 `%3A` 后，原生接口白名单没有解码，详情显示 unsupported API path。修复提交 `333a06b` 在校验前解码单个路径段，并测试拒绝注入额外参数和编码斜线。重打包独立 `HPS Final Validation.app`（`dev.hps.final-validation`），最终 build ID `5aeeffb9c6138494acb0136b50bab2f9503daae555157e024e368467da085bbb`；原生 7 项测试、Clippy 和构建通过。

修复后实际窗口暂停自动刷新，打开连接 `2dc1ec89-447b-44cd-a6b8-936706395a56:00000000000000000004`：源端口 64123、目标 56528、入口 validation-http、组 validation、固定节点、规则默认 r4，而当前运行参考版本为 r5；详情实际显示上传/下载各 183 B。点击断开后先显示包含唯一连接 ID 的确认范围；确认后显示主动断开、management_disconnect、活动数 0，测试客户端也收到实际 EOF。窗口布局已经检查，没有用 API 成功代替窗口操作。

显式代理的窗口门槛已满足。依赖 #16 的 Linux 进程归属、透明连接与内核/用户态代际尚未完成，待 T15 #20 联合验证后再关闭本 Issue。
