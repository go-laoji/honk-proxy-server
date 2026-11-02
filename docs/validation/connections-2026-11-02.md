# T13 业务连接与流量验收

Issue：[#18](https://github.com/go-laoji/honk-proxy-server/issues/18)。核心、API、CLI 和共享 MPA 已通过本机及真实代理验证。独立原生窗口仍受计算机工具锁屏报告阻断；Linux 全量回归正在执行，Issue 保持开放。Linux 透明流的进程与内核代际继续由 T15 联合验收。

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
| 全部共享控制台浏览器测试 | 16 passed | `/tmp/hps-t13-browser-all.log` |
| React 与 Node TypeScript 项目 | 通过 | `/tmp/hps-t13-browser-typecheck.log` |
| 完整前端格式检查 | 通过 | `/tmp/hps-t13-release-format.log` |
| 前端生产构建 | 通过 | `/tmp/hps-t13-browser-build.log` |
| 独立 CLI/Daemon 子进程 | 通过 | `/tmp/hps-t13-release-cli-process.log` |
| 最终 Native 桥单测 | 2 passed | `/tmp/hps-t13-release-native-tests.log` |
| 独立 macOS `.app` 打包 | 通过，43.34 MiB | `/tmp/hps-t13-release-native-build.log` |

可复现：`cargo test --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`pnpm typecheck`、`pnpm build`、`pnpm exec playwright test`；构建 headless 二进制后运行 `python3 tests/cli/connections.py`。前端产物更新必须与读取嵌入资源的 Rust 编译分开执行。

首次新增浏览器夹具将测试凭据写成顶层 upstream 字段，严格配置校验拒绝该请求；改用实际的 `credentials` 表后通过。完整格式检查也发现既有 rules 浏览器测试格式不一致，已单独格式提交并重跑通过。

## 独立桌面与剩余门槛

`HPS Connection Validation.app`，bundle ID `dev.hps.connection-validation`，独立连接偏好；隔离服务 `127.0.0.1:17999` 无供应商且入口默认关闭。API、共享生产产物与打包应用对应 `web_build_id=169275ac5ccde608668c35ad9b695ced253ca58ed2a8c5093b87e18cd1468837`。原用户 `17900` 服务、原 `.app` 和用户连接偏好保持不动。

用户确认解锁后，本次 `cua.getApp` 仍返回 `The Mac is locked and automatic unlock could not unlock it`，没有成功执行原生窗口动作。桥测试与打包不能代替窗口实际验收，未据此关闭 Issue。
