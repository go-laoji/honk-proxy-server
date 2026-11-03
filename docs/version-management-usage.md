# cargo-release 版本管理与 GitHub Release

本项目以根 `Cargo.toml` 的 `[workspace.package].version` 为产品版本来源。`cargo-release` 负责提升版本、创建一次版本提交和统一的本地 `v<version>` Git 标签；**不发布 Cargo registry，不自动 push**。

## 工具与版本范围

需要仓库 Rust 工具链、Python 3.11+（命令为 `python`）和固定的 cargo-release 1.1.7：

```sh
python --version
cargo install cargo-release --locked --version 1.1.7
cargo release --version
python -B scripts/versioning.py --check
```

本次开发环境已经安装 cargo-release 1.1.7。macOS 自带 `python3` 可能为 3.9，版本同步脚本使用标准库 `tomllib`，请让 `python` 指向 3.11 或更新版本。GitHub workflow 显式配置 Python 3.12。

| 位置 | 管理方式 |
| --- | --- |
| 主 workspace 的 17 个 crate | 继承根 `[workspace.package].version` |
| `src-tauri/Cargo.toml` | CLI crate 的一次 pre-release hook 同步 |
| `ebpf/hps-ebpf/Cargo.toml` | 同一 hook 同步；不调整 eBPF ABI 编号 |
| `package.json`、`src-tauri/tauri.conf.json` | 同一 hook 同步产品版本 |
| 三个 `Cargo.lock` | Cargo 离线更新本地 workspace 版本，检查第三方依赖没有变化 |

根 `release.toml` 配置 `publish = false`、`push = false`、共享版本和统一 `v{{version}}` 标签。Hook 仅配置在 `hps-cli`，避免每个 crate 重复执行。请使用整个 workspace 的命令，不用 `--package`/`--exclude` 发单个内部 crate。

## 提升版本与打标签

在干净的发布工作区操作。cargo-release 会检查未提交及未跟踪内容；当前主工作区保留的原始演示/编辑器文件不要为了发布而加入版本提交或删除，可在干净的源码副本中完成发布操作。

首次使用该源码副本时，先准备三个构建单元的锁定依赖索引与缓存；同步 hook 后续使用离线模式，不借版本提升升级第三方依赖：

```sh
cargo fetch --locked --manifest-path Cargo.toml
cargo fetch --locked --manifest-path src-tauri/Cargo.toml
cargo fetch --locked --manifest-path ebpf/Cargo.toml
```

先预览，再执行所选择的一种提升：

```sh
# 默认 dry-run：不改文件、不创建提交或 tag
cargo release patch --workspace

# 例如 0.1.0 -> 0.1.1；同步所有产品版本、锁文件并创建本地 v0.1.1
cargo release patch --workspace --execute

# 也可选择 minor / major，或直接给出预发行版本，替代上面的 patch 命令
cargo release 0.2.0-rc.1 --workspace --execute
```

执行后检查生成的版本提交和标签。同步失败会停止发版；脚本会恢复自己写入的独立 manifest/锁文件，不执行 Git reset、删除用户文件或修改已有标签。cargo-release 自身已经写入的根版本改动可能仍保留，需要先检查失败原因及 diff，再决定如何重试。

```sh
python -B scripts/versioning.py --check --tag v0.1.1
git show --stat HEAD
git show --no-patch v0.1.1
```

确认要触发 GitHub 构建后，由使用者显式推送对应分支与标签，例如：

```sh
git push origin HEAD
git push origin v0.1.1
```

`cargo-release` 不会代为执行这些 push，也不处理 Git Flow finish 或合入 main。仅创建本地标签不会触发 GitHub Actions。

## 标签构建自动发布 GitHub Release

`Cross-platform builds` 在版本标签 push 时会先校验 tag 与所有产品版本一致，再构建共享 Web 和五组原生 CLI/Daemon、Desktop。全部成功后，专用 `github-release` job 获得 `contents: write`，普通构建 job 仍为只读。

该 job 下载本次运行的十份原生 Artifacts，逐个校验源码提交、产品版本、目标架构、Web Build ID 和 SHA256。它将编译产物的原始字节同步到同名 GitHub Release：

- 五份 CLI/Daemon 压缩包及外部 `.sha256`。
- 五份 Desktop 安装包（Windows NSIS、Linux deb、macOS dmg）。
- 十份用包类型和 target 区分的 `*-manifest.json`，避免同名清单互相覆盖。
- `release-manifest.json` 和针对全部附件的总 `SHA256SUMS`。

正常共 27 个附件。原生文件名和字节不变，命令行包的 `-ci` 后缀也保留；总校验和使用实际发布的文件名。汇总结果还保存为 `release-<tag>` Actions Artifact。查看 **Releases → 对应版本 → Assets** 下载，或继续使用 Actions Artifacts。

新 Release 先创建为 draft，附件上传、远端 SHA256 确认全部成功后自动转为公开发布，不需要额外手动发布草稿。`-rc.1` 等预发行版本标为 prerelease。分支 push、PR 和普通手动 workflow 不创建 Release；失败或缺失任一平台产物也不会新建公开 Release。

发布中断时保留 draft，重新运行失败的发布 job 会跳过字节一致的附件并补齐缺失附件。已发布版本也允许补齐缺失附件；遇到同名但不同字节的附件则失败，不使用 `--clobber` 或删除原附件。若全量重建同一 tag 产生不同字节，应检查差异并发新版本，不能把不同产物静默替换到原标签下。

## 构建能力与验证边界

发布到 GitHub Release 不表示通过完整 Linux 内核发行门槛。当前通用 CI 包保持 `release_qualified: false`，不附带 eBPF 对象；完整 eBPF 包仍走 [既有发行流程](release-usage.md)。Windows 持久系统服务/系统代理尚未实现；Windows 安装包未云签名，macOS 只做 ad-hoc 签名、未公证。

本次只配置和验证流程，不在主工作区提升 `0.1.0`、创建产品标签、推送或真实发布 Release。集成测试在自有临时 Git 仓库中实际运行 cargo-release，发布接口使用无网络夹具：

```sh
python -B -m unittest discover -s tests/release-management -p 'test_*.py' -v
python -B -m unittest discover -s tests/ci -p 'test_*.py' -v
```

所有开发进程仍设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`，版本管理和 Release job 不安装系统服务或操作系统网络配置。

参考：[cargo-release 配置与 hook](https://github.com/crate-ci/cargo-release/blob/main/docs/reference.md)、[代表 crate 的单次 hook](https://github.com/crate-ci/cargo-release/blob/main/docs/faq.md)、[GitHub Release 创建](https://cli.github.com/manual/gh_release_create)、[附件上传](https://cli.github.com/manual/gh_release_upload)。
