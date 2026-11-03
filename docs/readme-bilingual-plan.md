# 中英文 README 编写计划

对应 [Issue #34](https://github.com/go-laoji/honk-proxy-server/issues/34)。基于当前 develop `bf22ada`，在 `feature/bilingual-readme` 编写，完成后保留本地供审核，不 push、不 finish；Issue 保持待审核。

## 交付

- 重写 `README.md` 为中文项目入口，新增对应的 `README.en.md`，提供相互切换链接。
- 说明用途、适用场景、已实现功能及平台能力边界，区分管理面可用与代理出口就绪。
- 使用 Mermaid Markdown 图展示 CLI/Daemon/Desktop/Web、应用服务、数据存储、代理引擎与 Linux eBPF 的关系；列出各目录和内部 crate 的职责。
- 给出发布包快速开始、各组件用法、源码编译、配置持久化、显式代理接入、系统服务/服务器部署、升级备份与版本发布的可执行入口。
- 以文档索引链接详细操作手册，保留已验证能力和未启用能力的边界，避免将历史验收记录当作产品入门步骤。

## 权威来源与重点纠正

以 `apps/cli` / `apps/daemon`、`crates/config` / `crates/system` / `crates/application`、`src-tauri`、`web/console/pages.manifest.json`、构建/安装脚本和工作流为准。现有 README 中“托盘/系统服务仍待后续交付”、过时的 x86 构建说明等需按当前实现纠正；Windows 运行包、系统服务支持和 eBPF 发行包须分别说明。

## 验证

- 核对真实 CLI `--help`、默认配置、管理端口、代理监听端口、存储路径和服务命令。
- 校验双语章节/关键命令对应、所有仓库相对链接、Markdown 和 Mermaid 语法；只读帮助/自有临时目录演练，不操作宿主系统服务或网络配置。
- 不把本次未运行的编译或其他平台安装写成已验收结果；不运行旧网络写入测试、不开启透明接管、不改变已安装 Desktop bundle 或用户偏好。

## 本地交付状态

中英文正文、Mermaid 架构图与验证已完成，详见 [验证记录](validation/readme-bilingual-2026-11-03.md)。当前保留在本地功能分支等待用户审核，不 push、不 finish，Issue #34 保持打开。
