# T14 后台任务与结构化事件验收

对应 [Issue #19](https://github.com/go-laoji/honk-proxy-server/issues/19)，实现和使用见 [任务与事件说明](../tasks-events-usage.md)。全 Rust 服务与共享 React/Tailwind MPA，不增加登录、账号或权限验证。

## 已交付行为

- 迁移 0008 将供应商、健康、配置激活的事实源在同一事务投影为统一任务与有界结构化事件；历史来源按创建时间混合导入。单调任务版本、原始 source_ref、实际配置版本/指纹、失败/跳过/结果不确定/中断与等待重启分别呈现，不用当前版本补写未知历史。
- 通用预检/诊断受服务所有权、16 个工作槽位和关闭注册边界控制。受控阻塞预检、调用方取消、子任务 panic、容量耗尽和关闭等待均验证；调用方退出不提前释放实际工作持有的锁或许可。
- 异步配置提交按实例 UUID、request_id、动作与完整输入指纹约束；同请求返回原任务，改动作/正文/备注/版本产生冲突，旧实例提交拒绝且不创建任务。任务输入不持久化，结果为有界安全摘要或资源引用。
- 任务查询提供类型/状态/来源引用/资源/请求/时间过滤及有界分页。配置、健康链接携带期望实例，API 与原生桥拒绝跨实例解析旧数字 ID。MPA 的 URL 支持刷新恢复与明确解除实例限制。
- SSE 从持久快照及完整游标恢复，过滤无命中也推进 checkpoint；窗口过期、超前游标与实例变化明确 reset。服务订阅上限 16，每批最多 200；任务终态/事件分别最多保留 10000 条。通用结果不超过 64 KiB，Rust 解码器限制帧/缓冲并拒绝异常帧推进游标。
- 原生订阅限制 4 个工作器、每窗口一个当前订阅及 16 个窗口意图；目标/文档代际、提前取消、晚到启动、重复 ACK、15 秒 ACK 截止、实际 TCP 释放均有回归。每帧等待对应确认，旧目标不能回流到新页面。
- 管理操作只记录实际 TCP 对端、固定动作/资源/结果和关联 ID，忽略伪造转发头；没有已认证操作人假设。配置正文/备注、供应商 URL 查询、认证头、凭据与业务数据不进入正常事件。

## 自动化结果

| 范围 | 实际结果与证据 |
| --- | --- |
| macOS aarch64 工作区 | 155 passed / 0 failed / 0 ignored；`/tmp/hps-t14-macos-tests.log` |
| macOS 全工作区 all-target Clippy | `-D warnings` 通过；`/tmp/hps-t14-macos-clippy.log` |
| Linux aarch64 工作区 | 154 passed / 0 failed / 4 ignored，all-target Clippy 通过；`/tmp/hps-t14-linux-regression.log` |
| 最终资源构建后的 macOS API / web-assets | 13 passed；`/tmp/hps-t14-final-api-assets.log` |
| 最终资源构建后的 Linux API / web-assets / Clippy | 13 passed、Clippy 通过；`/tmp/hps-t14-linux-final-assets.log` |
| CLI / Daemon 实际子进程 | 幂等候选 r2、改输入冲突退出 7、旧实例提交不登记、真实 SSE、同地址重启 reset 与 SIGINT 清理通过；`/tmp/hps-t14-epoch-cli.log`，可运行 `python3 tests/cli/tasks.py` 复现 |
| 最终 Native 测试 / Clippy | 7 passed、all-target Clippy 通过；`/tmp/hps-t14-final-native-tests.log`、`/tmp/hps-t14-final-native-clippy.log` |
| React + Node 类型 / 完整格式 | 通过；`/tmp/hps-t14-final-typecheck.log`、`/tmp/hps-t14-final-format.log` |
| 最终 Chromium 全集 | 18 passed；`/tmp/hps-t14-final-browser-all.log` |
| 最终生产前端 / 独立原生包 | 构建通过；`/tmp/hps-t14-final-console-build.log`、`/tmp/hps-t14-final-native-build.log` |

Linux 使用独立源码快照、Rust 1.98.1 Alpine 和单构建任务，未清理用户 Docker 资源。4 个忽略项为实际特权 DNS/eBPF 的子进程/捕获入口，本轮不计为执行；已有 6.12/7.0 实际内核证据保留，T15 的完整业务透明数据面未由本轮代替。

浏览器新增用例使用独立随机端口、唯一实例名和内存库，无供应商网络或购买：真实配置提交/重复请求/异输入冲突，关联配置只读预览，健康来源引用及独立文档跳转；事件暂停期间列表冻结、恢复补取、同地址新实例清空旧列表与详情、旧链接拒绝、显式解除实例限制、真实管理对端展示。390px 截图 `/tmp/hps-t14-tasks-mobile.png` 与 `/tmp/hps-t14-events-mobile.png` 已检查，等待移动侧栏完成隐藏后无溢出。

## 实际 Desktop 窗口

先使用独立 `HPS Task Validation.app` 连接 18000：窗口提交 `native-t14-validate`，任务 #1 达 completed/v3/valid；相同请求再次提交仍为 #1，关联事件展示 queued/running/completed。暂停事件流后，CLI 在同一服务提交 `native-t14-stream`，暂停列表保持为空，恢复后补齐任务 #2 的三条事件及完整游标。

窗口发现清空筛选时空查询字符串 `events?` 被原生白名单拒绝。修改前端仅在存在参数时附加 `?`，重打包为独立 `HPS Final Validation.app`（`dev.hps.final-validation`，44.27 MiB）。最终 frontend build ID 为 `5aeeffb9c6138494acb0136b50bab2f9503daae555157e024e368467da085bbb`，连接独立 18001 服务时窗口确认同一构建。无筛选事件页面正常进入实时订阅；CLI 提交 `native-final-stream` 后，实际窗口收到 5 条管理/任务事件，详情显示真实 TCP 对端 `127.0.0.1:64956`、task_submit/accepted，无身份冒充。

从该事件页面通过连接页切换至 17999 真实代理验收服务，窗口显示新目标及其业务数据，没有混入原服务事件。原生生命周期/实际 TCP 取消的细节由 7 项原生回归覆盖。目标切换后进一步验证 T13 的编码连接详情与真实断开，见 [连接验收](connections-2026-11-02.md)。用户原应用、17900 服务、yun-free 配置和连接偏好均未改动。

## 如实保留的范围

本轮不做已付费提取自动重放，不把探测任务完成等同于所有节点健康，也不提供尚未实现的取消/重试。历史年龄清理、统计和导出由 T18 继续实现；当前统一任务与事件按固定条数保留，不能把 retention.task_days/activity_days 当作已生效的这些表的年龄策略。T15 的 Linux 完整 TCP/UDP、进程与规则代际继续是独立发布门槛。

两次早期浏览器失败分别是测试对隐式标签的精确定位及夹具默认停用健康检查；修正为按控件角色定位，并明确先启用隔离健康策略。原生窗口发现的空查询与编码连接 ID 是产品问题，分别修复后重新打包并用真实窗口验收。早期失败不算作最终通过。
