# T12 节点管理验收

Issue：[#17](https://github.com/go-laoji/honk-proxy-server/issues/17)。后端、CLI 和共享页面已实现；**独立桌面窗口，以及与 T13 逐连接列表的关联仍待验收，Issue 保持开放**。

## 实现与真实用例

迁移 0007 保存独立 CAS 操作状态；当前与退休引擎共用准入门和真实拨号/套接字计数。静态配置与供应商租约统一分页、过滤、排序、导入预览，来源/批次、寿命、健康和人工限制分别呈现。软删除仅作用于稳定节点/租约 ID，不改其他批次或原始到期时间。来源摘要显示真实库存、水位与最近提取异常。`nodes.html` 与 `node-detail.html` 是独立 MPA，浏览器和桌面复用同一页面。

关键测试：

- Engine `actual_tunnel_drains_after_disable_and_generations_share_gate_and_live_count`：实际 HTTP/HTTPS/SOCKS5 上游，新 CONNECT 被拒绝，旧隧道继续传输/半关闭，热引擎共用计数并最终归零。
- Storage `operational_state_survives_restart_and_rejects_stale_writes`、`lease_control_requires_exact_batch_and_preserves_other_lease_and_original_deadlines`：持久恢复、双版本 CAS、精确批次归属与不续期。
- Application `inventory_separates_expiry_health_operational_controls_and_exact_lease_ownership`：真实库存、来源水位、组/能力/分页过滤、脱敏、禁用准入与补池库存一致；`caller_cancellation_does_not_abandon_accepted_node_action` 验证客户端离开后操作仍完成。
- API `real_http_import_preview_explicit_partial_apply_query_controls_and_cas`：真实 HTTP typed client 贯穿预览、显式部分选择、激活、查询、动作与冲突；凭据不出现在返回或输入 Debug 中。

## 已运行检查

| 环境 / 检查 | 结果 | 日志 |
| --- | --- | --- |
| macOS aarch64 workspace tests | 129 passed / 0 failed / 0 ignored | `/tmp/hps-t12-release-macos-tests.log` |
| macOS all-target Clippy `-D warnings` | 通过 | `/tmp/hps-t12-release-macos-clippy.log` |
| Linux aarch64 workspace tests + all-target Clippy | 128 passed / 0 failed / 4 ignored，Clippy 通过 | `/tmp/hps-t12-release-linux-tests.log` |
| 独立 CLI/Daemon 实际子进程 | 通过：配置 r2、操作版本 4、库存 2、冲突退出码 7 | `/tmp/hps-t12-release-cli-validation.log` |
| React 类型与生产构建 | 通过，最终独立详情 MPA 已构建 | `/tmp/hps-t12-mpa-build.log` |
| 最终 MPA/节点浏览器回归 | 6 项全部通过 | `/tmp/hps-t12-mpa-browser-tests.log` |
| 最终 Native 桥与打包 | 1 项单测通过，独立 `.app` 已构建 | `/tmp/hps-t12-mpa-native-tests.log`、`/tmp/hps-t12-mpa-native-build.log` |

Linux 忽略项为需要特权内核环境的 DNS 核心/应用验收及子进程入口，不能算作本次执行；已有实际 6.12/7.0 内核证据见 DNS/eBPF 验收记录。此前一次 lint 读取旧的依赖接口，重建带新文件时间的独立源码快照后，完整测试与 lint 通过。

浏览器此前 15 项完整集全部通过（`/tmp/hps-t12-browser-all.log`）。新增来源水位夹具时遗漏通用提取必需的 `count_parameter`，该轮 14 通过、1 失败；补齐后节点用例通过（`/tmp/hps-t12-release-node-browser.log`）。最终针对实际详情 MPA 导航重跑 console+nodes 6 项全通过，覆盖独立文档、嵌入字节一致、真实代理 200→禁用 503→恢复 200、过期 CAS、URL 恢复、来源水位与健康跳转。390px 截图 `/tmp/hps-t12-nodes-mobile.png` 已实际检查，无横向溢出。未将夹具失败隐藏为产品成功。

后续 T13 的完整 `pnpm typecheck` 发现节点浏览器测试跨 composite 项目导入 Web 类型，触发 TS6307；已将测试需要的消费契约保留在 Node 测试项目内，不改变产品 DTO 或页面。Web 与 Node 两个 TypeScript 项目检查重新通过，日志 `/tmp/hps-t13-platform-typecheck.log`。

CLI 可复现：先构建 `hps-cli`/`hps-daemon`，运行 `python3 tests/cli/nodes.py`。使用随机端口、唯一实例名和临时内存库，清理自身子进程和目录；不启动桌面、不调用供应商。

## 桌面及未关闭门槛

独立 `HPS Node Validation.app` 已构建，路径 `src-tauri/target/debug/bundle/macos/HPS Node Validation.app`；bundle ID `dev.hps.node-validation`，独立连接偏好。临时验收服务 `127.0.0.1:17998` 无供应商、入口与静态节点默认禁用。API 与打包页面的 `web_build_id` 均为 `d1bf3cb9f011186c86c01479e90f30d9434e8376f804e4a5ded94e5a101911c7`。原用户 `17900` 预览与原桌面进程保持不动。

用户回复已解锁后，计算机工具仍返回 `Mac is locked and automatic unlock could not unlock it`；未操作窗口，打包和原生桥测试不能代替窗口验收。已异步报告，并继续不依赖桌面的工作。

逐连接记录、流量、主动断开和详情列表跳转由 T13 在同一登记表上接续。当前计数来自真实拨号/套接字，无演示连接。Linux 本机进程及内核/用户态代际一致性继续与 T15 联合验收。
