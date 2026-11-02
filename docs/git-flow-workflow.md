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

## 联合验收完成后的第二批

#16/#18/#20 在 52 项实际内核案例及 CLI/MPA 联合验收后已回复并关闭。随后按上述顺序逐个执行 `git flow feature finish`：

| 顺序 | Feature | 原 tip | develop 合并提交 |
| --- | --- | --- | --- |
| 12 | routing-rules | 4c4216f | 7f749d5 |
| 13 | node-management | acec18a | 4b01cee |
| 14 | connection-observation | 26d5916 | d2add0d |
| 15 | tasks-events | 1b6540b | 67fa524 |
| 16 | linux-transparent | 4680e2c | a197cbe |

每个合并均有两个父提交，内容树等于相应原 feature tip，原 tip 作为第二父提交保留。`main` 仍为 `18d5899`，没有 push 或远端删除，原始未跟踪文件未变。当前从 develop 开启 `feature/desktop-service` 推进 #21；上述第一批 retained 列表作为历史快照保留在 JSON 的 `retained_at_first_batch`，当前待 finish 列表为空。

第三批：#21 按安全窗口与明确 OS 写入边界验收回复并关闭后，使用标准 finish 完成 `desktop-service`：原 tip `0321fe7`，双父合并 `43363f7`，树一致。随后回到 `release-packaging`，通过 merge 同步 develop；没有先 finish 后继分支、手动删除其他 feature、push 或改动 main。

第四批：#22 回复发行候选的实际证据和平台限制后，`release-packaging` 原 tip `27b0060` 独立 finish 为双父合并 `82f6f5b`，内容树完全一致；本地候选标签 `v0.1.0-rc.1` 指向原 tip。随后通过 start 创建 `observability-history` 处理 #23。main 未改动、未 push；没有批量删分支。

第五批：#23 历史观测、诊断、告警与受限任务控制完成并关闭后，`observability-history` 原 tip `c6f0755` 独立 finish 为 `ec69bc3`，双父与内容树一致。用户授权余下 #24/#25/#26 使用独立 managed worktree 并行开发和交叉审查；只有各项集成验收完成后按该顺序逐项 finish，不批量删除分支。

第六批：#24 在协议、订阅代际和来源寿命修复与组合验收后关闭，`protocol-subscriptions` 原 tip `9639b69` 独立 finish 为双父合并 `d3123be`，内容树一致，已归档对应 managed worktree，原 tip 由第二父保留。#25/#26 继续按该顺序收尾；用户随后明确允许编译没有报错时先合并代码，仍在进行的 x86 静态 release 额外发行验收单独记录为 pending，不提前计为通过。

第七批：按用户“编译没有问题可以先合并”的最新指示，#25 在核心组合验收完成、x86 额外静态发行验证明确 pending 的边界下回复并关闭。`backup-desktop-diagnostics` 原 tip `fedc475` finish 为 `92e95a6`，双父与内容树一致；完成后 detach，保留该 checkout 供正在运行的冻结发行构建使用。随后在 #26 分支 merge develop，按顺序处理最后的储备契约分支；不因收尾而中断构建或提前归档其目录。

第八批：#26 储备范围与兼容契约复核后回复并关闭，`future-platform-contracts` 原 tip `5d8377a` finish 为 `4a45a58`。已回到主工作区 develop；累计 22 次 finish 的双父、第二父原 tip、内容树一致及 develop 可达性逐条复核通过，所有原提交记录保留，当前无 feature 引用。`main` 仍为 `18d5899`，未 push/删除远端/发布。x86 额外静态发行验收按用户授权继续在冻结输入上运行，与已完成的代码合并分开记录；构建结束前保留其 detached worktree。此后只做有范围的文档与验收维护提交，不为记录最终 merge SHA 再制造空功能分支。
