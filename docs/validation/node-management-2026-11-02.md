# T12 节点管理验收

Issue：[#17](https://github.com/go-laoji/honk-proxy-server/issues/17)。后端、CLI、共享页面、真实逐连接关联与独立桌面窗口已完成验收。

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

## 实际桌面验收

独立 `HPS Node Validation.app` 已构建，路径 `src-tauri/target/debug/bundle/macos/HPS Node Validation.app`；bundle ID `dev.hps.node-validation`，独立连接偏好。临时验收服务 `127.0.0.1:17998` 无供应商、入口与静态节点默认禁用。API 与打包页面的 `web_build_id` 均为 `d1bf3cb9f011186c86c01479e90f30d9434e8376f804e4a5ded94e5a101911c7`。原用户 `17900` 预览与原桌面进程保持不动。

此前计算机工具持续报告锁屏。2026-11-02 解锁期间，最终 T13 的 `HPS Connection Validation.app` 连接 17999 隔离服务，完成以下实际操作：

- 导入一条带合成凭据的 HTTP 节点与一条不支持的 vmess 行；预览显示有效 1 / 无效 1，确认按钮停用。显式选择第 1 行并重新预览后，确认导入并激活 r3。返回内容没有展示凭据。
- 使用该节点建立真实 CONNECT 隧道；窗口显示业务连接计数 1。点击“排空”后人工状态为排空中，可准入数归零；旧隧道继续回显，新隧道返回 503。点击“解除限制”后，新隧道恢复 200。
- 节点详情显示“上游凭据：已配置（隐藏）”、组 validation 与实际业务连接链接；点击链接进入携带 node 筛选的独立 connections 文档，显示真实源端口、节点和字节数。

后续 `HPS Final Validation.app`（build ID `5aeeffb9c6138494acb0136b50bab2f9503daae555157e024e368467da085bbb`）进一步完成该节点真实连接详情和断开。原应用、17900 预览与其配置均未修改。

T13 已在同一登记表上接入逐连接记录、流量、主动断开和详情跳转。浏览器真实隧道在节点详情显示 1 条活动业务，进入筛选后的连接列表，并可返回该节点的独立详情文档；见 [连接验收](connections-2026-11-02.md)。计数覆盖拨号、业务交换和实际隧道生命周期，无演示连接。Linux 本机进程及内核/用户态代际一致性继续与 T15 联合验收。
