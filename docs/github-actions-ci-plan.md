# GitHub 三系统 CI 实施计划

对应 [Issue #31](https://github.com/go-laoji/honk-proxy-server/issues/31)。从 `develop` 通过 `git flow feature start github-actions-build` 开发，按切片提交；本任务不主动 push、finish 或合入 main，保护原始未跟踪文件。

## 当前构建契约

- Rust 1.98.1、Node 24.15.0、pnpm 12.10.1，分别以仓库工具链文件、`package.json` 和锁文件为准。
- `pnpm build` 生成包含 18 页及 SHA256 清单的共享 Web；Daemon 和 Desktop 必须使用相同字节。
- CLI 为 `hps`，Daemon 为 `hpsd`，Windows 可执行文件带 `.exe`；Desktop 使用独立 `src-tauri/Cargo.toml` 和锁文件。
- 现有 `release.yml` 联合真实 Linux 内核验收后才调用 headless/Desktop 发行打包器；没有 Windows，也没有 push 触发的普通 CI。普通构建不能假称通过 eBPF 发行门槛。
- Windows 当前没有持久系统服务或系统代理集成；生成 Windows 包不等于新增这些运行能力。

## 交付切片

1. **工作流与构建脚本**：新增 `build.yml`，支持 push（main/develop/feature）、PR、`v*` tag 和手动运行；Web 单次构建，Windows x64、Linux x64/ARM64、macOS ARM64/Intel 原生构建。显式安装仓库 Rust 工具链和 Linux WebView 依赖，使用锁文件、固定 Action SHA、只读仓库权限、有界超时及并发取消。
2. **可下载产物**：普通 CI CLI/Daemon 包包含默认配置、来源/能力清单及内部和外部 SHA256；Windows 使用 zip，Unix 使用保留执行位的 tar.gz。Desktop 包含同一 Daemon 资源，Windows 输出 NSIS、Linux 输出 deb、macOS 输出 dmg。每个平台独立上传具名 Artifacts，记录源码提交、目标架构与 Web Build ID。普通 Linux CI 包不附带未验收 eBPF，也不替代原正式发行包。
3. **验证与文档**：运行工作流语法检查、打包回归测试、共享 Web 构建和本机独立目录的原生构建/元数据检查。记录本地与远端执行边界，补充触发、下载及复跑说明；回复实际结果后关闭 Issue。

## 安全与验收边界

所有构建和元数据检查设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`。不运行系统网络写入、安装服务、启动 Desktop 窗口或旧网络测试二进制。输出仅放入本任务独立的 `target/ci-build`，不覆盖原 `target/release` 或 `src-tauri/target`。远端工作流需在获授权推送后执行，本地验证不记为 Windows/Linux GitHub runner 验收。

实现依据：[GitHub runner 清单](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)、[Tauri CI](https://v2.tauri.app/distribute/pipelines/github/) 和 [Tauri 构建依赖](https://v2.tauri.app/start/prerequisites/)。
