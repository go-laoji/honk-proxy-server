# 仅版本标签触发 CI：本地验证

对应 [Issue #35](https://github.com/go-laoji/honk-proxy-server/issues/35) 和 [实施计划](../ci-tag-only-plan.md)。`feature/ci-tag-only` 基于已验收的 README 分支 `329b643`，产品版本保持 `0.1.1`。

## 改动与边界

- `build.yml` 与 `release.yml` 的独立入口均仅保留 `push.tags: ['v*']`。普通分支、PR、手动事件与非版本标签不触发。
- `headless.yml` / `desktop.yml` 仍只提供 `workflow_call`，唯一调用者为标签触发的 `release.yml`。
- 对四份 YAML 与基线逐项解析比较，除顶层 `on` 外所有字段一致：构建矩阵、打包、实际内核门槛、权限、Artifact 上传和 GitHub Release 发布逻辑均未改变。
- 中英文 README、CI/发行/版本手册已同步新触发范围；用户的本轮 finish/push 授权补记到 README 计划，保留先前本地审核阶段的记录。

## 已执行检查

| 检查                       | 结果                                                                                                                                         |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| actionlint 1.7.11          | 四个工作流全部通过                                                                                                                           |
| YAML 事件检查              | 每条独立入口 12 组，共 24 组通过；覆盖 main/develop/feature/release/hotfix/名称似 tag 的分支、PR、手动事件、非版本 tag、正式与预发行版本 tag |
| 复用调用与配置比较         | 唯一调用链正确，所有既有 jobs 及其他工作流字段未变化                                                                                         |
| `tests/ci`                 | 6 项通过                                                                                                                                     |
| `tests/release-management` | 10 项通过；真实 cargo-release 在临时仓库执行，GitHub 发布使用无网络夹具                                                                      |
| 版本一致性                 | `python -B scripts/versioning.py --check` 通过，仍为 `0.1.1`                                                                                 |
| README                     | 两版格式、相对链接/图片与目录锚点通过；内嵌 TOML 经发布版 CLI 校验通过                                                                       |
| Git diff                   | `git diff --check` 通过                                                                                                                      |

依据 [GitHub Actions 官方标签过滤语义](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#onpushbranchestagsbranches-ignoretags-ignore)，只设置 tags 时不会匹配分支推送。本轮不为验证创建/重推版本 tag，不将本地事件检查称为已执行远端版本构建。

## 收尾范围

用户已明确授权依次 finish README 与 CI 功能分支，并推送 develop。执行后在 Issue #34/#35 回填实际合并提交、远端一致性与未触发构建的核对结果，再关闭已完成 Issue。

保留独立 Git Flow 合并记录；不合入 main、不变更 v0.1.1 标签、不推送本地测试仓库中的标签。主工作区原始未跟踪文件核对保持原样；未运行宿主网络写入测试或系统服务操作。
