# Git Flow 分支收尾规则与记录

用户于 2026-11-02 明确要求：每项新任务开始前检查现有功能分支，按顺序 finish 可以收尾的分支，避免仅 finish 最新 feature 后再删除其他分支。此要求是对本会话此前“不主动 feature finish”的后续明确授权。未完成的功能继续保留，推送与发布仍不在这次操作范围。

## 后续任务开始前

1. 检查当前分支、工作树、worktree、Git Flow 配置、分支继承关系与 GitHub Issue 的实际状态。保护现有未提交内容及用户原始未跟踪文件。
2. 核对每个 feature 的实际交付/验收范围。Issue 关闭不代表可以跳过仍未完成的物理祖先分支；避免后续 feature 先合入，使较早 feature 失去独立的收尾合并记录。
3. 从最早的可收尾 feature 开始逐项执行 `git flow feature finish <name>`，保留每项独立合并提交和原 feature tip。遇到未完成门槛时保留该分支及其后续继承分支。
4. 每次核对合并父节点、内容树和剩余分支。不要手动批量删除分支，不通过 rebase/reset 改写既有提交，不使用此次未授权的 push 或远端删除。
5. 当前环境是 Git Flow 0.4.1：有多个待合入提交时使用 `--no-ff`；只有一个时会 fast-forward。收尾前需把必要验收记录作为真实内容提交，确保代码和验收可追溯，不制造空提交。`-F` 在此版本的 finish cleanup 还会删除远端分支，因此本次及未明确授权远端操作时均不使用。
6. 回到仍在开发的 feature，以 merge 同步 develop 的收尾历史，恢复并核对原工作。随后继续已授权的 Issue 开发。

## 2026-11-02 已执行

原有 16 个 feature 是串行继承关系。已依次完成下列 11 次独立 finish，均为双父合并，合入后的内容树与该 feature 原 tip 一致，没有冲突：

| 顺序 | Feature | 原 tip | develop 合并提交 |
| --- | --- | --- | --- |
| 1 | proxy-architecture-plan | e89cc84 | 1d8fe2b |
| 2 | runtime-foundation | 5816766 | 564e05e |
| 3 | linux-preflight | 7ba9e9b | b71bc53 |
| 4 | explicit-proxy | 89fa028 | 744d55c |
| 5 | config-revisions | d9834a6 | 968199f |
| 6 | provider-leases | a9f652c | c3e7962 |
| 7 | pool-replenishment | 54d2486 | d0c28b0 |
| 8 | health-probes | 9ec4353 | 758c036 |
| 9 | linux-ebpf-core | 31a6b40 | d30acf1 |
| 10 | dns-pipeline | f3907ea | 605231a |
| 11 | group-selection | 2dbd4eb | efdfa54 |

规划分支交付的是规划和 Issue 拆分，不表示上级 Epic #5 已完成；T03 的 readonly preflight 与后续真实 eBPF core 分别保留了收尾记录。开发任务 #6–#15 已实时核对为 closed。

继续保留的 5 个 feature：

- `routing-rules`：#16 的 Linux 进程与原子发布联合验收尚未完成。
- `node-management`：#17 已关闭，按实际继承顺序等待 routing-rules 收尾。
- `connection-observation`：#18 仍有联合验收和规则依赖门槛。
- `tasks-events`：#19 已关闭，按实际继承顺序等待前面的连接分支收尾。
- `linux-transparent`：#20 仍在开发。

已回到 `feature/linux-transparent`，通过 `8464c56` 同步 develop。同步前后该活动分支的已提交源码内容树完全一致；6 个 Linux UDP 验收工作文件已精确恢复，21 个用户原始未跟踪文件逐个校验未变。`main` 未改动，没有 push、远端删除或手动批量删分支。标准 finish 只删除了各次已合并的本地 feature 引用，每个原 tip 仍作为对应 merge 的第二父节点保留。

完整 SHA、实时 Issue 状态和校验摘要见 [收尾记录](validation/git-flow-finishes-2026-11-02.json)。后续完成 Linux 联合门槛后，继续按 routing-rules → node-management → connection-observation → tasks-events → linux-transparent 的顺序检查和收尾。
