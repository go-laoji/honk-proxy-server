# 中英文 README 本地验证

对应 [Issue #34](https://github.com/go-laoji/honk-proxy-server/issues/34) 和 [编写计划](../readme-bilingual-plan.md)。当前为用户审核稿，分支 `feature/bilingual-readme` 基于 develop `bf22ada`，产品版本仍为 `0.1.1`。

## 文档范围

- 重写根 `README.md` 为中文入口，新增 `README.en.md`；互相提供语言切换链接。
- 两版对应介绍用途、已实现功能与边界、组件和 Mermaid 架构、发布包启动、代理配置、CLI/Daemon/Web/Desktop/eBPF 用法、源码编译、部署服务、备份升级、验证、CI 与版本发布。
- 从真实源码、默认配置、工具链清单、打包脚本、工作流和实际 CLI 帮助核对行为。纠正旧 README 中 Desktop 托盘/服务尚待交付等过时内容，区分通用 GitHub Release 包与完整 eBPF 验收包。
- 仅修改两份 README、本计划和本记录。无运行代码、产品版本、锁文件、工作流或网络配置变更。

## 静态检查

| 检查           | 结果                                                                                                                                 |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Prettier       | 两版 `--check` 通过                                                                                                                  |
| 双语结构       | 各 14 个二级标题、13 个三级标题；23 个代码块的类型与数量相同                                                                         |
| 相对路径与目录 | 各 41 个仓库相对链接存在，13 个目录锚点均匹配对应标题                                                                                |
| TOML 示例      | 两版代码块分别提取后，均由发布版 `hps config check` 校验通过                                                                         |
| Mermaid        | 使用 Mermaid 11.12.2 解析并在临时 headless Chromium 中实际渲染；两图均有 17 个节点，已检查布局；图保持 Markdown 源码，无新增图片依赖 |
| 产品版本       | `python -B scripts/versioning.py --check` 通过，仍为 `0.1.1`                                                                         |
| Git diff       | `git diff --check` 通过                                                                                                              |

核对 `hpsd` 参数、CLI 全局参数、各命令组及服务/备份参数时，只执行 `--help`，没有触发系统服务、备份恢复或内核加载。

## 发布版快速开始演练

使用已有 v0.1.1 Apple Silicon headless 包的程序（源码标记 `6cc259f`），验证 README 中与该版本对应的启动和查询路径。此演练没有重新编译当前文档分支。

- 在自有临时目录生成基础 TOML，校验成功；重复 `hps init` 拒绝覆盖既有文件。
- 设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1` 和独立的 `HPS_CLIENT_STATE_DIR`，在自有临时工作目录启动 `hpsd --ephemeral`，监听临时 loopback 端口。
- 默认配置不含业务入口、上游或 Linux 捕获；`/healthz` 返回 200，`/readyz` 返回 503，`/console/` 返回 200，与 README 描述一致。
- 20 条目标查询通过：`status`、`version`、`capabilities`、`config show`、`config workspace`、`provider list`、`subscription list`、`inbound list`、`node list`、`group show`、`rule show`、`health show`、`dns show`、`connection list`、`task list`、`event list`、`linux preflight`、`linux diagnostics`、`linux capture status`、`ui`。`ui` 未传 `--open`，不打开浏览器。
- `config export --path ...` 写入临时文件后，再经本机 `config check` 校验通过。
- 演练结束只向本次启动的 Daemon 发 SIGTERM，进程正常退出，退出码 0。

## 验证边界与待审核状态

本次为文档改动，未重新执行全量 Rust/Web/Desktop 编译、跨平台安装、真实上游转发或实际 eBPF 内核矩阵；README 的这些操作入口以当前代码、帮助及既有对应验收记录为依据，不将列出的命令当作本轮已执行结果。

未安装/启停宿主系统服务、未修改已安装 Desktop、用户偏好或活动网络配置，未运行旧网络写入测试。仅用临时进程、临时文件和 loopback API 演练。文档分支有本地小步提交，未 push、未 finish、未创建版本标签。Issue #34 保持待用户审核。
