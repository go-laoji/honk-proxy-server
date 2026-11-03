# 中英文 README 本地验证

对应 [Issue #34](https://github.com/go-laoji/honk-proxy-server/issues/34) 和 [编写计划](../readme-bilingual-plan.md)。当前为用户审核稿，分支 `feature/bilingual-readme` 基于 develop `bf22ada`，产品版本仍为 `0.1.1`。

## 文档范围

- 重写根 `README.md` 为中文入口，新增 `README.en.md`；互相提供语言切换链接。
- 两版对应介绍用途、已实现功能与边界、组件和 Mermaid 架构、发布包启动、代理配置、CLI/Daemon/Web/Desktop/eBPF 用法、源码编译、部署服务、备份升级、验证、CI 与版本发布。
- 从真实源码、默认配置、工具链清单、打包脚本、工作流和实际 CLI 帮助核对行为。纠正旧 README 中 Desktop 托盘/服务尚待交付等过时内容，区分通用 GitHub Release 包与完整 eBPF 验收包。
- 仅修改两份 README、本计划、本记录和 `docs/screenshots/` 下的四张 PNG。无运行代码、产品版本、锁文件、工作流或网络配置变更。

## 静态检查

| 检查           | 结果                                                                                                                                                                          |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Prettier       | 两版 `--check` 通过                                                                                                                                                           |
| 双语结构       | 各 17 个二级标题、13 个三级标题；24 个代码块的类型与数量相同                                                                                                                  |
| 相对路径与目录 | 各 45 个仓库相对链接存在，16 个目录锚点均匹配对应标题                                                                                                                         |
| TOML 示例      | 两版代码块分别提取后，均由发布版 `hps config check` 校验通过                                                                                                                  |
| Mermaid        | 使用 Mermaid 11.12.2 解析并在临时 headless Chromium 中实际渲染；每版含 5 节点调用链路图和 17 节点架构图，四图均通过并检查布局；Mermaid 图保持 Markdown 源码，运行截图另行添加 |
| 产品版本       | `python -B scripts/versioning.py --check` 通过，仍为 `0.1.1`                                                                                                                  |
| Git diff       | `git diff --check` 通过                                                                                                                                                       |

核对 `hpsd` 参数、CLI 全局参数、各命令组及服务/备份参数时，只执行 `--help`，没有触发系统服务、备份恢复或内核加载。

## 用户审核意见补充

- 两版增加固定 HTTP 入口 → 规则/池选点 → 可用上游 → 目标站点的请求链路，说明爬虫与浏览器无需随池中节点变化频繁修改代理地址。
- 核对 `configs/hps.proxy.example.toml` 及引擎路由/拨号实现，明确 `17900` 为默认管理端口，样例 HTTP 业务入口为 `17901`；规则选定候选范围后才按健康、寿命、容量及策略选点。
- 明确新连接选点、连接复用/会话固定、Direct/Reject 与无合格节点拒绝的边界，没有承诺每请求换 IP、迁移已有连接或重放请求。
- 文末加入程序完全由 AI 编写、不保证没有 Bug 及 GitHub Issues 问题/建议反馈入口，包含中英文对应说明。
- 调用链路补充阶段重新检查了格式、双语结构、链接/锚点、TOML 与四张 Mermaid 图；该阶段没有重新启动业务代理或修改宿主代理配置。

## 发布版快速开始演练

使用已有 v0.1.1 Apple Silicon headless 包的程序（源码标记 `6cc259f`），验证 README 中与该版本对应的启动和查询路径。此演练没有重新编译当前文档分支。

- 在自有临时目录生成基础 TOML，校验成功；重复 `hps init` 拒绝覆盖既有文件。
- 设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1` 和独立的 `HPS_CLIENT_STATE_DIR`，在自有临时工作目录启动 `hpsd --ephemeral`，监听临时 loopback 端口。
- 默认配置不含业务入口、上游或 Linux 捕获；`/healthz` 返回 200，`/readyz` 返回 503，`/console/` 返回 200，与 README 描述一致。
- 20 条目标查询通过：`status`、`version`、`capabilities`、`config show`、`config workspace`、`provider list`、`subscription list`、`inbound list`、`node list`、`group show`、`rule show`、`health show`、`dns show`、`connection list`、`task list`、`event list`、`linux preflight`、`linux diagnostics`、`linux capture status`、`ui`。`ui` 未传 `--open`，不打开浏览器。
- `config export --path ...` 写入临时文件后，再经本机 `config check` 校验通过。
- 演练结束只向本次启动的 Daemon 发 SIGTERM，进程正常退出，退出码 0。

## 运行截图补充

用户要求增加运行时截图后，使用 v0.1.1 Apple Silicon 发布包（源码 `6cc259fedf0248c781607e10f75826773b25cf3d`，`source_dirty=false`，Web Build ID `79068c7ecd7fe61a5b30f4f48f4970a67d75d074aaf0fd47261a7b520379d270`）启动实际 Web 控制台，在临时 headless Chromium 中截图。两版 README 引用同一份图片，提供中英文图注并标明本地示例环境；界面当前为中文。

| 图片                                              | 尺寸        | 内容                                                                         |
| ------------------------------------------------- | ----------- | ---------------------------------------------------------------------------- |
| [overview.png](../screenshots/overview.png)       | 1440 × 1100 | 管理运行、代理就绪、3 个库存节点、2 个可准入节点、3 条活跃连接及实际双向流量 |
| [nodes.png](../screenshots/nodes.png)             | 1440 × 1476 | 两个已检测健康的本地上游及一个明确禁用的节点，完整保留节点卡片               |
| [rules.png](../screenshots/rules.png)             | 1440 × 1100 | 本地流量匹配 `crawler-pool`、示例域名明确拒绝，以及当前运行版本              |
| [connections.png](../screenshots/connections.png) | 1440 × 2006 | 三条真实 CONNECT 隧道的命中规则、出口与流量，完整保留连接记录                |

截图不是概念图，也没有拦截 API、改写 DOM 或伪造健康/连接数据：

- 独立 `--ephemeral` Daemon、独立客户端目录，管理与业务入口均绑定临时 loopback 端口；开发进程设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`。
- 两个自有本地 HTTP 上游只允许访问本次创建的回环目标；健康目标也在回环地址，不使用默认公网探测站点，不配置供应商采购或订阅。
- 两节点均实际取得健康成功证据，`/readyz` 返回 200。规则将回环连接指向轮询分组 `crawler-pool`，三条 CONNECT 分别由两个上游转发（2 + 1），持续传输并回显测试字节。
- API 记录确认三个连接均为 `open` / `transferring`，命中 `local-crawler-traffic`，使用 `crawler-pool`；字节计数与页面显示来自真实转发。
- 四页无浏览器脚本错误或页面错误提示。逐张检查了文字、关键记录、图片尺寸与内容，四图共 941752 字节；两版图片路径均通过本地链接检查。
- 截图后关闭本次创建的浏览器、连接与测试服务，只向自有 Daemon 发送 SIGTERM，正常退出码 0。没有安装服务、加载 eBPF、修改宿主系统代理/DNS/路由或访问用户原代理配置。

## 验证边界与审核阶段状态

本次为文档改动，未重新执行全量 Rust/Web/Desktop 编译、跨平台安装、公网代理出口验收或实际 eBPF 内核矩阵；README 的这些操作入口以当前代码、帮助及既有对应验收记录为依据，不将列出的命令当作本轮已执行结果。

未安装/启停宿主系统服务、未修改已安装 Desktop、用户偏好或活动网络配置，未运行旧网络写入测试。仅用临时进程、临时文件、loopback API 与自有回环代理流量演练。文档分支有本地小步提交，未 push、未 finish、未创建版本标签。Issue #34 保持待用户审核。

上述为 README 审核阶段记录。用户后续已明确授权修改 CI 后依次 finish 并推送 README 与 CI 分支；最终收尾和远端核对结果回填 [Issue #35](https://github.com/go-laoji/honk-proxy-server/issues/35)，不创建新版本标签。
