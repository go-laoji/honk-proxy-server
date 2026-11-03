# 手动添加代理节点验收（#30）

- 日期：2026-11-03（Asia/Shanghai）。
- 任务：[Issue #30](https://github.com/go-laoji/honk-proxy-server/issues/30)，分支 `feature/manual-proxy-nodes`，功能提交 `b55615d`。
- 最终共享 Web 构建标识：`79068c7ecd7fe61a5b30f4f48f4970a67d75d074aaf0fd47261a7b520379d270`。

## 实现

代理节点页面增加独立“手动添加”按钮。表单支持 HTTP/HTTPS/SOCKS5、IP/域名/IPv6、端口、可选标识、用户名密码、启用状态和实际到期时间；链接快速填写支持 SOCKS5h、IPv6 和百分号编码凭据。密码默认隐藏，可临时显示；链接填入后清空，凭据不进入页面 URL 或浏览器持久存储。打开表单定位到输入区域，移动布局无横向溢出。

点击添加复用现有 `/nodes/import/preview`、`/nodes/import/apply`，使用严格 JSON、预览摘要和所见配置版本，不引入第二套节点存储。静态配置持久化并热激活，后续继续遵守统一健康、分组和选点规则。重复标识拒绝覆盖；配置冲突刷新版本、保留表单，需用户重新提交。添加成功清空表单并定位新节点，批量导入独立保留。

Tauri 与浏览器共享此页面，已有原生 bridge 允许上述接口，无须新增 native 命令。本轮验证浏览器与实际 Daemon，未重新打包或安装 Desktop。

## 真实浏览器与转发

运行 `pnpm test:e2e tests/console/manual-nodes.spec.ts tests/console/nodes.spec.ts`，最终 **3/3 通过**。测试启动真实 Rust Daemon，操作持久配置接口，未使用 mock API。

- 逐项添加认证 HTTP 节点，通过真实健康检测（含出口回显），刷新后库存仍在；hps 入口实际经过验证上游认证的自有本地代理访问本地目标，业务连接选中所添加节点，凭据未泄露到目标或节点查询。
- HTTP/HTTPS/SOCKS5 默认端口和自定义端口保留；SOCKS5h 链接、百分号凭据、IPv6、到期时间、配置禁用、自动标识、无认证节点均通过。重复 ID 不改变库存；模拟配置版本竞争后显示冲突并保留表单，重新提交成功。
- 原批量导入、部分行选择、无效项拒绝、库存筛选恢复、业务准入限制和详情链接回归通过。1440px Light、390px 移动布局及 Dark 画面已检查；移动端无横向溢出。

测试调试中修正了选择控件定位方式及成功提示等待条件；最终没有遗留失败。

## SQLite 重启验收

另启独立临时数据目录的 Daemon（不使用 `--ephemeral`），从真实浏览器表单添加认证节点 `manual-persistence-proof`。记录版本 r2，先完成一次 hps 入口认证转发；只关闭该临时 Daemon，再使用同一目录及初始空上游 TOML 重启。数据库仍保持 r2 和节点，浏览器刷新可见，认证状态保留，第二次实际转发成功。自有上游累计认证请求 2 次，证明密码在重启后仍能使用，不仅是列表标记保留。

最终独立运行记录：

```json
{
  "before_restart_revision": 2,
  "after_restart_revision": 2,
  "browser_added": true,
  "node_and_credentials_preserved": true,
  "proxy_traffic_before_and_after_restart": true,
  "authenticated_upstream_requests": 2,
  "passed": true,
  "daemon_stopped": true
}
```

## 检查与边界

`pnpm typecheck`、`pnpm format:check`、`pnpm build` 和 `git diff --check` 均通过。最终浏览器测试与独立重启验收使用相同的上述 Web 构建标识。Rust 核心、数据库 schema、供应商逻辑及现有配置版本契约保持复用，没有新增迁移。

所有进程设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`；测试只有自有回环服务、临时 SQLite 数据目录和合成凭据，不调用真实供应商或重新测试用户长效节点。所有临时 Daemon 已停止；没有改宿主 Wi-Fi/DNS/路由/系统代理、原服务、Desktop bundle 或原始未跟踪文件。分支保留，不主动 push/finish。
