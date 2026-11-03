# 本项目开发与验收边界

- 后端使用 Rust，CLI/Daemon 独立于 Desktop；前端共用 React/Tailwind 多页面产物；不引入项目登录、帐号或权限验证。
- Git Flow 按用户约定 start 分支并小步提交。用户已明确授权：验收通过的分支按依赖顺序逐个 finish，保留独立合并记录，不跳过较早分支，不主动 push 或合入 main。GitHub 开发任务须回复真实结果后再关闭。
- **不得修改开发宿主 Mac 的活动网络配置，也不得使其 Wi-Fi、DNS、路由或代理断网。** 2026-11-02 的代理写入测试曾误写默认配置，旧的 `real_systemconfiguration_private_preferences_commit_and_restore` 验收结论无效，禁止重新运行旧测试二进制。
- macOS 宿主验收只允许网络配置只读检查及无 OS 写入的事务/恢复测试；`cargo test` 必须拒绝创建可写系统配置会话。开发应用必须设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`。需要实际网络配置写入的验收只能在确认隔离的虚拟机内进行，不能把临时文件路径、mock 或 ignored 当作隔离证明。
- Linux 网络与真实 eBPF 验收使用自有容器/虚拟机、独立 cgroup/netns 和明确的对象归属，不修改宿主 Mac 网络。保护用户原服务、Desktop bundle、偏好和原始未跟踪文件。
- 操作系统网络配置和恢复副本含本机隐私，不得提交仓库或发送到 Issue。发现事故先停止写入、只读诊断并保留副本；不得自动恢复旧网络配置而再次影响连接。

## 版本管理与 cargo-release

- 使用 **cargo-release 1.1.7** 统一管理产品版本和 Git 标签；根 `Cargo.toml` 的 `[workspace.package].version` 是唯一版本来源。主 workspace 的内部包均继承它，不单独发版，不使用 `--package` / `--exclude` 绕开整体同步，也不手工逐文件修改版本。
- 根 `release.toml` 必须保持 `publish = false`、`push = false`、`shared-version = true`、统一版本提交及 `v{{version}}` 标签。**不发布到 crates.io 或其他 registry，不自动 push**；版本变更必须创建对应 Git 标签，不能只改版本号而漏打标签。
- `apps/cli/Cargo.toml` 中的单次 pre-release hook 调用 `scripts/versioning.py`，同步 Desktop、eBPF、`package.json`、Tauri 配置及三个 `Cargo.lock`。不要在其他 crate 重复配置该 hook，不要直接调用 `--hook` 绕过 cargo-release。产品版本提升不改变 eBPF ABI，也不升级第三方依赖。
- 工具链使用仓库固定的 Rust；Python 必须为 **3.11+**，且 `python` 命令可用。安装 cargo-release 时固定版本：`cargo install cargo-release --locked --version 1.1.7`。
- 只有用户要求发版或提升版本时才执行实际版本变更，普通代码/文档提交不自动发版。按实际改动选择 `patch` / `minor` / `major`，也可使用用户指定的完整 SemVer；一次操作只选择一种版本提升方式。
- 发版前完成相应验证并提交发布范围内的改动，在干净的发布工作区执行。用户原始未跟踪文件不得为发版而删除或加入版本提交；有此类文件时，按当前工作区约定使用干净的源码副本或独立工作区。

首次准备发布工作区时，先缓存三个构建单元的锁定依赖；版本 hook 后续离线同步锁文件：

```sh
cargo fetch --locked --manifest-path Cargo.toml
cargo fetch --locked --manifest-path src-tauri/Cargo.toml
cargo fetch --locked --manifest-path ebpf/Cargo.toml
```

先检查版本一致性并执行 dry-run，再在已授权的发版范围内执行选定的版本提升。以下为 patch 示例，minor / major / 显式版本替代 patch 使用：

```sh
python -B scripts/versioning.py --check
cargo release patch --workspace
cargo release patch --workspace --execute
```

- 不带 `--execute` 的命令仅预览，不修改产品文件或创建提交/标签；带 `--execute` 才会同步版本、创建一次版本提交和统一的本地标签。
- 执行后检查实际版本、diff、版本提交和标签指向。三个锁文件只允许预期的本地包版本变化；若出现第三方依赖变化，应停止发版并检查原因。标签须与产品版本匹配，并指向对应版本提交。

```sh
HPS_RELEASE_TAG=v0.1.2 # 示例，替换为本次实际生成的标签
python -B scripts/versioning.py --check --tag "$HPS_RELEASE_TAG"
git show --stat HEAD
git show --no-patch "$HPS_RELEASE_TAG"
```

- 标签保留预发行段（如 `v0.2.0-rc.1`），不包含 `+build` 元数据；不能靠修改 build metadata 重复使用既有标签。不强制移动、删除或重写已发布版本标签。
- 推送遵循用户明确授权的范围，已授权的操作无需重复询问。只推送选定发布分支及对应版本标签，不使用 `git push --tags` 批量带出无关标签。cargo-release 不代替 Git Flow finish，也不意味着获准合入 main。
- GitHub Actions 仅在推送 `v*` 标签时构建；普通分支 push、PR 和手动事件不启动构建。三平台构建全部成功后自动发布同名 GitHub Release 附件；完整 eBPF 验收使用独立标签工作流，不把普通 CI 包视为完整内核验收包。
- 同步失败时先检查日志和 diff。Hook 会恢复自己写入的独立 manifest/锁文件，但 cargo-release 已写入的根版本可能保留；不要自动 reset、清理用户文件或盲目重试。Release 附件同名但字节不同必须停止并检查，不覆盖已发布产物，应修复后发布新版本。
- 版本操作继续设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`，不安装服务或修改宿主网络。完整操作说明见 [版本管理手册](docs/version-management-usage.md)。
