# GitHub 三系统 CI 验证记录（2026-11-03）

对应 [Issue #31](https://github.com/go-laoji/honk-proxy-server/issues/31)，功能分支 `feature/github-actions-build`。

## 交付

`build.yml` 使用五组原生 runner：Windows x64、Linux x64/ARM64、macOS Apple Silicon/Intel。共同 Web 只构建一次；每个平台先上传 CLI/Daemon 压缩包，再构建并上传 Desktop 安装包，附源码、目标、Web Build ID、能力边界及 SHA256。支持分支 push、PR、`v*` tag、手动触发；Artifacts 保留 14 天。原完整 eBPF 发行流水线和门槛不变。

## 实际通过

- actionlint v1.7.11：仓库全部四个工作流的 YAML、表达式、依赖和 runner 检查通过；本机未安装 ShellCheck，未计入 ShellCheck 验收。
- `pnpm typecheck`、`pnpm format:check`、`pnpm build`：通过，共 18 个页面。
- `python3 -B -m unittest discover -s tests/ci -p 'test_*.py' -v`：6 项通过，覆盖 zip/tar 内容、Unix 执行权限、内外校验和、五种目标的元数据拒绝路径、Web 篡改/路径/缺页，以及无图形打包环境与网络保护强制设置。
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check` 和 `git diff --check`：通过。
- macOS ARM64：从干净提交 `1095e11dd78b0ca79b3e5ff0efb17cf239bde691` 实际完成 headless 和 Desktop release 构建。
- headless 压缩包解压后逐项复核 SHA256，实际执行解压后的两个 build-info 和默认配置校验：通过。
- Desktop 实际生成 ad-hoc 签名的 `.app` 和 `.dmg`；从 `.app` 中执行无窗口 build-info，与 headless 的来源和 Web ID 一致；内置 hpsd 字节 SHA256 一致，上传目录清单完整，`hdiutil verify` 通过。

共同 Web Build ID：`79068c7ecd7fe61a5b30f4f48f4970a67d75d074aaf0fd47261a7b520379d270`。

| 实际产物 | 大小（字节） | SHA256 |
| --- | ---: | --- |
| `hps-headless-0.1.0-aarch64-apple-darwin-ci.tar.gz` | 14,783,721 | `a38cc030cf756ac39408b67fb267660a42b2ba11060481fd3bdb3fea6f7108e1` |
| `honk-proxy-server_0.1.0_aarch64.dmg` | 13,648,883 | `50afb7b0cd014ca0fcca5c3c8e7dda00c6982fab3443eb305fe5e276484087dd` |

输出位于 `target/ci-build/artifacts/headless/` 和 `target/ci-build/artifacts/desktop/`，未覆盖原始 `target/release`、`src-tauri/target` 或已安装应用。本记录在冻结构建提交之后单独提交；产物仍如实标记上面的实际源码提交。

## 打包问题与修复

首次本地 DMG 排版等待 Finder 自动化，已终止该次构建的 osascript，由打包器卸载其自有临时镜像；不申请图形权限。脚本改为强制 `CI=true` 并移除 `TAURI_BUNDLER_DMG_IGNORE_CI`，以跳过 Finder 排版。随后发现 Tauri 的 DMG-only 构建会清理 `.app`，改为同时生成 `app,dmg`，保留 `.app` 以核对内置 Daemon。上述最终干净提交已经完整复跑通过；所有自有打包镜像均已卸载。

## 未执行与安全边界

- 未 push、未 finish、未合入 main，尚无本轮 GitHub Actions run。Windows、Linux、Intel Mac 的配置已加入，远端实际构建和运行均未验收；不得以本机 ARM64 构建代替。
- 普通 CI 包标记 `release_qualified: false`，Linux 不附带 eBPF 对象，不声明通过真实内核验收。Windows 持久系统服务/系统代理集成仍未实现；macOS 未云签名或公证。
- 全程设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`。未安装/启动系统服务、打开 Desktop 窗口或写入 OS 网络配置，未执行旧的系统网络写入测试。
- 10 个原始未跟踪文件的 SHA256 逐一核对未变。构建清单不含本机网络配置、恢复副本或凭据。
