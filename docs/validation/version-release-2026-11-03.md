# cargo-release 与 GitHub Release 验证记录

日期：2026-11-03。对应 [Issue #32](https://github.com/go-laoji/honk-proxy-server/issues/32)，分支 `feature/cargo-release-github`。

## 交付与验证输入

- `release.toml`：共享版本、单次提交、统一本地 `v<version>` 标签，`publish = false`、`push = false`。实际安装和验证 cargo-release 1.1.7。
- CLI 的一次 pre-release hook：同步 Desktop/eBPF 的 Cargo manifest、前端/Tauri JSON 和三个 Cargo 锁文件，拒绝第三方依赖变动，失败时恢复自身写入的文件。
- `build.yml`：所有构建先检查版本/标签一致性；仅 tag push 且五组原生构建全部成功时进入具有 `contents: write` 的专用 GitHub Release job。
- Release 汇总：检查全部十份原生产物、源码、版本、架构、Web 与 SHA256；发布 27 个唯一名称附件，包含十份包、五份压缩包外部校验和、十份逐平台清单、总清单和总校验和。源码和构建字节不重新标记，eBPF qualification 保持 false。

核心交付提交：`735236d`（版本同步）、`b10dd1b`（附件发布）、`f655765`（build metadata 与标签命名对齐）。当前产品版本保持 `0.1.0`。

## 实际通过

1. **工具实际配置**：`cargo release config --manifest-path Cargo.toml` 解析为 `publish = false`、`push = false`、共享版本、统一 tag 和合并版本提交；`cargo release --version` 为 1.1.7。
2. **完整仓库快照**：将版本同步切片 `735236d5a3d8641f2922741331a3d4bc3d7c8f4b` 的全部已跟踪源码复制到独立临时 Git 仓库，隔离当前工作区、原始未跟踪文件和实际 remote。17 个主 crate 的真实 `cargo release patch --workspace` 预览没有改变文件、HEAD 或标签；实际执行只调用一次 hook，生成一次版本提交及本地 `v0.1.1`，所有产品位置和三个锁文件一致，第三方依赖不变。临时仓库 remote 指向自有不存在路径，自动 push 不可能成功；整个操作未触及真实仓库的版本或标签。
3. **回归测试**：`python -B -m unittest discover -s tests/release-management -p 'test_*.py' -v`，8 项通过。使用真实 cargo-release 验证 patch、`0.2.0-rc.1` 和 `0.3.0+build.7`，并检查单次提交、tag 位置、干净工作区、三个锁文件及 dry-run；构建 metadata 保留在产品版本中，tag 遵循工具的无 build metadata 规则。
4. **发布接口夹具**：上述测试同时验证十份原生产物汇总为 27 个附件，损坏/缺失/错版本/错源码/不一致 Web/错误 qualification 被拒绝；首次 draft 创建、上传校验后发布、相同附件重跑、上传中断保留 draft 并续传均通过。同名不同字节会在写入前被拒绝，403 不被当作不存在的 Release。全部发布测试使用受控 GH 接口夹具，没有网络发布。
5. **既有打包回归**：`python -B -m unittest discover -s tests/ci -p 'test_*.py' -v`，6 项通过。
6. **配置与静态检查**：actionlint v1.7.11 对全部工作流检查通过（未调用本机不存在的 ShellCheck）；版本一致性检查、Python AST 和 `git diff --check` 通过。
7. **GitHub 只读接口**：实际验证不存在 Release 的 404 响应解析，没有执行远端写入。已有基线 [Actions run 41565151546](https://github.com/go-laoji/honk-proxy-server/actions/runs/41565151546) 的源码为 `4c50f9c`，实时状态 success；这是已有构建的结果，不是本次新增 Release job 的运行结果。

## 当前边界

- 本次没有提升真实产品版本、增加真实产品标签、push、finish 或合入 main；真实仓库仍只有此前的 `v0.1.0-rc.1` 标签。新增发布流程需要在用户选择版本并推送包含本配置的 tag 后运行，尚未进行真实 GitHub Release 发布验收。
- 主工作区的 10 个原始未跟踪文件逐项核对 SHA256 不变；Cargo 锁文件、原 Desktop bundle、服务与偏好均未改变。
- 全程 `HPS_FORBID_HOST_NETWORK_CHANGES=1`，没有运行应用、系统网络写入测试、服务安装/启动或宿主网络配置修改。
- 通用附件仍不包含 eBPF 对象，也不代表通过完整真实内核发行验收。Windows 未云签名，macOS 仅 ad-hoc 签名、未公证；与 GitHub Release 的发布位置分别记录。
