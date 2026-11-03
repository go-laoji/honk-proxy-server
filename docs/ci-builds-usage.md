# GitHub Actions 三系统构建与下载

工作流为 `.github/workflows/build.yml`，在 Actions 中显示为 **Cross-platform builds**。提交到 `main`、`develop`、`feature/**`、`release/**`、`hotfix/**`，创建 PR 或推送 `v*` 标签时自动执行，也支持 **Run workflow** 手动选择分支。工作流文件必须先推送到 GitHub；手动按钮还要求工作流存在于默认分支。

## 产物矩阵

| 系统 | 架构 / target | CLI + Daemon | Desktop |
| --- | --- | --- | --- |
| Windows | x64 / `x86_64-pc-windows-msvc` | `.zip`，含 `hps.exe` / `hpsd.exe` | NSIS `.exe` 安装包 |
| Linux | x64 / `x86_64-unknown-linux-gnu` | `.tar.gz` | `.deb` |
| Linux | ARM64 / `aarch64-unknown-linux-gnu` | `.tar.gz` | `.deb` |
| macOS | Apple Silicon / `aarch64-apple-darwin` | `.tar.gz` | `.dmg` |
| macOS | Intel / `x86_64-apple-darwin` | `.tar.gz` | `.dmg` |

各架构使用对应原生 GitHub runner。Web 只在一个 job 中执行 `pnpm build`，然后所有原生构建下载同一 `ci-shared-web`；Tauri 的二次前端构建在本次生成的配置中关闭。构建前后检查 Web 文件 SHA256，实际执行二进制的 build-info 核对目标架构、源码提交、干净源码标记和 Web Build ID。

在仓库 **Actions → Cross-platform builds → 某次运行 → Artifacts** 下载：

- `headless-<target>`：CLI/Daemon 压缩包、外部 `.sha256`、`manifest.json`、`SHA256SUMS`。压缩包内部也有清单和逐文件校验和，并附默认配置和本说明。
- `desktop-<target>`：对应安装包、包含 Desktop/Daemon 编译元数据的 `manifest.json` 和 `SHA256SUMS`。
- `ci-shared-web`：共同 Web 构建及 `web-manifest.json`。

Artifacts 保存 14 天。每个平台先上传 headless，再构建 Desktop；Desktop 构建失败时，同平台已经成功上传的 headless 包仍可下载。矩阵使用 `fail-fast: false`，单个平台失败不会取消其他平台。

## 使用与能力边界

解压后先核对 SHA256，再直接运行 `bin/hps`、`bin/hpsd`，Windows 使用对应 `.exe`。例如在解压目录中：

```sh
./bin/hps build-info
./bin/hps config check config/hps.toml
./bin/hpsd --config config/hps.toml --data-dir ./data
```

Windows PowerShell 使用：

```powershell
Get-FileHash .\hps-headless-*-ci.zip -Algorithm SHA256
.\bin\hps.exe build-info
.\bin\hps.exe config check config\hps.toml
.\bin\hpsd.exe --config config\hps.toml --data-dir .\data
```

运行 CLI/Daemon 无需 Node、Python、Tauri 或 Desktop。Linux CI 包在 Ubuntu 24.04 上链接 glibc，运行时需要兼容的 glibc；它不是静态 musl 便携包。Desktop 仍需要对应系统的 WebView/图形依赖。Windows 持久系统服务与系统代理集成尚未实现，可直接运行 Daemon 或让 Desktop 连接已运行的目标服务。

所有这些产物标记 `artifact_kind: ci-build` 和 `release_qualified: false`。普通 Linux CI 不打包 eBPF 对象，也不声明透明接管已经通过内核验收；需要完整 eBPF 的发行包继续使用 [发行流程](release-usage.md) 中的 `release.yml`，满足真实双架构/双内核门槛。Windows 安装包未做云签名；macOS 仅 ad-hoc 签名，未做 Apple 公证。工作流上传 Artifacts，不创建 GitHub Release。

## 本地复跑

先使用锁文件安装依赖并构建 Web，再提交相关源码。在与 target 匹配的原生系统执行：

```sh
pnpm install --frozen-lockfile
pnpm build
python3 -B -m unittest discover -s tests/ci -p 'test_*.py' -v
# 示例为 Apple Silicon；其他系统替换为表中的原生 target。
python3 -B scripts/build-ci.py headless --target aarch64-apple-darwin
python3 -B scripts/build-ci.py desktop --target aarch64-apple-darwin
```

构建工具链来自 `rust-toolchain.toml`、`package.json`、`packaging/toolchains.json`，全部 Cargo/pnpm 命令使用锁文件。Windows 也可使用 `python` 调用脚本。输出固定在独立的 `target/ci-build`；脚本拒绝覆盖已存在的同类 artifacts 目录，复跑前自行保存并移走该目录。不要删除原 `src-tauri/target` 或已安装应用。

构建始终设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`，仅执行无窗口的编译元数据检查，不安装或启动系统服务，不写系统代理、DNS 或路由。开发应用运行时仍需设置该变量。

参考：[GitHub runner 清单](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)、[Artifacts](https://github.com/actions/upload-artifact)、[Tauri CI](https://v2.tauri.app/distribute/pipelines/github/) 与 [Linux 依赖](https://v2.tauri.app/start/prerequisites/)。本地检查与 GitHub 实际执行结果应分别记录，未运行的平台不能记为验收通过。
