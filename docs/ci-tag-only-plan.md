# 仅版本标签触发 CI

对应 [Issue #35](https://github.com/go-laoji/honk-proxy-server/issues/35)。按用户授权，在 README 分支基础上建立 `feature/ci-tag-only`；修改并验证后依次 finish 两个分支，推送 develop，不创建新版本、不改写已有标签、不合入 main。

## 变更

- `build.yml` 与 `release.yml` 仅保留 `push.tags: ['v*']`，移除分支、PR 和手动入口。
- `headless.yml` / `desktop.yml` 保持 `workflow_call`，只能通过完整标签验收链调用；不改变打包、内核验收或 GitHub Release 附件发布逻辑。
- 同步中英文 README、CI、发行与版本使用说明，避免文档继续承诺分支/PR/手动触发。

## 验证与收尾

- actionlint 检查四个工作流；按 YAML 语义检查事件、标签过滤、复用调用链和原有 jobs 未变化。
- 验证普通分支、PR、手动运行及非版本 tag 不触发，版本与预发行 tag 触发；运行已有 CI/发布管理测试和版本一致性检查。
- 保护主工作区原始未跟踪文件，保留 README 和 CI 独立 Git Flow 合并记录；推送后核对远端 develop、原版本 tag 和 main，确认此次普通提交没有新构建。
- 不为验证创建或移动远端 tag，不执行宿主网络写入或系统服务操作。

依据：[GitHub Actions 标签过滤语法](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#onpushbranchestagsbranches-ignoretags-ignore)。
