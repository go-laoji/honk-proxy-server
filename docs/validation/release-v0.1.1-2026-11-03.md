# v0.1.1 发布与 draft 查询修复

对应发布授权及 [Issue #33](https://github.com/go-laoji/honk-proxy-server/issues/33)。

## 已发布结果

- 原功能 `cargo-release-github` 通过标准 Git Flow finish 合入 develop，独立双父合并为 `c4b4e05b5d25f360cb7a0fb396076894fcb6b0d5`，内容树等于原 tip `befeaf3`，原历史保留。
- cargo-release 1.1.7 从该 develop 提升到 `0.1.1`，版本提交为 `6cc259fedf0248c781607e10f75826773b25cf3d`。仅八个版本/锁文件变化，17 个主 crate、Desktop、eBPF 和前端一致，未发布 Cargo registry。
- 带注释的 tag 对象为 `a947f58726d5fb46c689ad8f8f78fb09e75cc376`，`v0.1.1^{commit}` 指向上述版本提交。develop 和 tag 已经原子推送，后续恢复没有改写 tag 或二进制源码标记。
- [GitHub Release v0.1.1](https://github.com/go-laoji/honk-proxy-server/releases/tag/v0.1.1) 已于 **2026-11-03 12:24:06（Asia/Shanghai）** 发布，Release ID `432271557`，`draft=false`、`prerelease=false`。
- 五组原生构建全部通过：Windows x64、Linux x64/ARM64、macOS Intel/Apple Silicon。27 个附件均为 uploaded，逐项比对远端 API 的 SHA256、大小与本轮 CI 原始附件完全一致；总清单记录的源码为 `6cc259f`、产品版本为 `0.1.1`，包含全部五个 target。
- [标签工作流 41573506947](https://github.com/go-laoji/honk-proxy-server/actions/runs/41573506947) 的失败发布 job 单独重跑后通过，五组原生构建结果保留，没有重新编译或重标记二进制。

## 首次远端发布问题

首次发布 job `125722982624` 完成十份原生产物汇总并上传 `release-v0.1.1`（Artifact `13204540447`，外层 ZIP SHA256 `ea67513c5f4766f83a06e127a64f80e4cbe14d6ffef8d77222723b89ffdb54b2`），随后创建了空 draft，但在回读时失败。

原因是 `/releases/tags/{tag}` 只返回已发布的 Release，新建 draft 对该 endpoint 仍为 404。此前夹具错误地让 draft 在 tag endpoint 可见，遗漏了这个真实 API 差异。

修复提交 `22807ca` 在 tag endpoint 返回 404 后，通过认证后的完整分页 Release 列表匹配 tag，支持初次 draft 和中断续传；403 等错误继续中止，重复匹配不会任意选择草稿。发布汇总 Artifact 允许覆盖本 job 自己的已验证备份，以支持发布重跑，不涉及 GitHub Release 附件覆盖。

## 恢复和验证

1. 按真实 API 行为修改夹具，新增 draft lookup 与重复匹配拒绝回归；10 项版本/发布测试、actionlint 和 diff 检查通过。
2. 只读查询实际草稿，确认修复后的发布器找到相同 Release ID `432271557`，其时附件数为零。
3. 下载同一标签工作流已经校验的 27 个附件，逐个验证全局 SHA256、源码和版本；使用修复后的发布器续传该 draft，远端 SHA256 全部确认后转为已发布。
4. 再次查询 Release，确认 27 个附件与原始字节完全一致、tag 对象及指向未变；只重跑原工作流的失败发布 job，发布重跑成功，没有删除、替换或重建已发布附件。

## 边界

当前产物保持普通构建的 `release_qualified=false` 和无 eBPF 对象标记，不替代完整真实内核发行验收。Windows 未云签名、macOS 仅 ad-hoc 签名且未公证。发布过程没有操作宿主网络配置、原有系统服务或已安装应用，main 未改动。

依据：[GitHub Release API](https://docs.github.com/en/rest/releases/releases#get-a-release-by-tag-name)、[官方 CLI 的 draft 查询实现](https://github.com/cli/cli/blob/trunk/pkg/cmd/release/shared/fetch.go)。
