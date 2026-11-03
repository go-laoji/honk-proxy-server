# 统一版本管理与 GitHub Release 实施计划

对应 [Issue #32](https://github.com/go-laoji/honk-proxy-server/issues/32)，功能分支 `feature/cargo-release-github`。本次配置发布流程，不提升当前 `0.1.0`，不推送、finish 或合入 main。

## 当前契约

- 主 Rust workspace 的 17 个 crate 继承根 `[workspace.package].version`；`src-tauri` 和 `ebpf` 是独立构建单元，分别持有版本与锁文件。
- `package.json`、`src-tauri/tauri.conf.json` 也持有产品版本。Tauri 与 Daemon 打包校验要求版本、源码及共同 Web 一致。
- `build.yml` 已按五组原生 runner 生成 Windows/Linux/macOS 的 headless 与 Desktop 产物，只有 Actions Artifacts，没有 GitHub Release 附件。
- 普通构建没有附带 Linux eBPF 对象，元数据 `release_qualified: false` 是内核验收边界；发布到 GitHub Release 不改变此结论。

## 交付切片

1. **cargo-release 与版本同步**：固定工具版本，根 `release.toml` 关闭 registry publish 和自动 push，共享版本、单次提交、统一 `v<version>` 标签；代表 crate 的单次 hook 同步 Desktop/eBPF、前端/Tauri 配置与三个 Cargo 锁文件。dry-run 不能修改文件或创建标签；提供独立的版本一致性检查。
2. **Release 汇总与发布**：只在 `v*` 标签 push 时运行写权限 job，且必须等待共同 Web 与全部五组原生构建成功。校验标签版本、源码、目标、共同 Web、十份清单及每个附件的 SHA256；消除通用文件名冲突，生成总清单与校验和。发布到同名 GitHub Release，预发行标签标记 prerelease，重跑可以补全附件，不覆盖已有不同字节的附件。
3. **验证与文档**：使用隔离临时 Git 仓库实际执行 cargo-release 的预览和版本提升，验证仅一次 hook/提交/tag、所有版本与锁文件同步、无 registry/push。用本地构建夹具与受控发布接口测试汇总、缺失/损坏/错版本拒绝、初次发布与重跑。运行 Actions 语法和现有打包回归检查，写验证记录并回复/关闭 Issue。

## 验收边界

不使用当前工作区试发版本，不创建本次产品版本标签或真实远端 Release；这些动作由用户后续选择版本并推送标签触发。所有工具与应用设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`，只做配置、版本与产物处理；不改宿主网络、原始应用/服务或 10 个原始未跟踪文件。

依据：[cargo-release 配置](https://github.com/crate-ci/cargo-release/blob/main/docs/reference.md)、[单次 hook 配置建议](https://github.com/crate-ci/cargo-release/blob/main/docs/faq.md)、[GitHub Release 创建](https://cli.github.com/manual/gh_release_create) 与 [附件上传](https://cli.github.com/manual/gh_release_upload)。
