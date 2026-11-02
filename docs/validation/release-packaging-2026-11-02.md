# T17 分包与发行候选验收

对应 [#22](https://github.com/go-laoji/honk-proxy-server/issues/22)。工程交付为可追溯、可复跑的发行候选；没有推送、发布 registry、云签名/公证或宣称所有平台已通过。安装命令见 [发行用法](../release-usage.md)，结构化事实见 [JSON](release-packaging-2026-11-02.json)。

## 产物与平台范围

实际编译源码为 `2b196752e0f69c430c13285199a153a1b3d9855c`，修正后的安装/打包源码为 `8c51b6199006b99c546279074a53f0db9a15ce3f`。两者所有 Rust/前端/锁文件/构建输入经 Git 比较完全相同，分别记录真实版本。共同 Web ID 为 `6ddd3f17e72bf05dd6bdcc9e2bc5ab13c6002297860cbfd5df208b463b5eca95`，18 个 HTML 均实际返回 200。

| 产物 | 实际结果 | 本地产物 |
| --- | --- | --- |
| macOS aarch64 headless | 安装、CLI、代理、保留配置、损坏拒绝通过 | `target/validation/release-candidate/macos/` |
| macOS aarch64 Desktop | release app 27.23 MiB；源码/Web/内嵌 hpsd 一致，ad-hoc 签名通过 strict 验证，离线元数据命令无窗口 | `target/validation/desktop-release/release/bundle/macos/honk-proxy-server.app` |
| Linux aarch64 musl headless | ELF 无 PT_INTERP；Debian 无 Node/显示运行；Alpine BusyBox 安装通过；附 ABI4 对象与真实报告 | `target/validation/candidate-2b19675/linux-arm64/` |
| Linux x86_64 musl headless | 仍在构建，未列为已通过发行目标；此前真实 x86 内核矩阵独立保留 | 不作为本轮通过依据 |
| Linux Desktop/deb、远端 GitHub Actions | 流水线已实现，尚未实际执行，不能冒称已通过 | 无已验收安装包 |

Desktop 打包使用独立 target，未覆盖用户原始 bundle。实际窗口只读验收与服务生命周期见 T16；本项只执行新包的 `--build-info`，不启动或操作系统代理。

## 已执行检查

- macOS workspace **185**、Native **9**、Chromium **18** 项通过；Linux 用户态 **192** 项通过。普通 Linux workspace 的 **30** 个特权/子进程入口 ignored 不计作通过；实际内核由下面的独立客体证据验收。
- macOS/Linux/Native Clippy、Rust fmt、Web 类型/格式/生产构建通过。关闭 Web 的 headless 编译独立通过，依赖树不引入 Tauri；精简构建不宣称包含控制台。
- Rust Web 验证器拒绝损坏/缺失/额外条目、错误 MPA 清单和 symlink，并把验证后的字节冻结至 OUT_DIR，避免前端重建与 Rust 嵌入竞争。包脚本执行真实离线元数据命令，拒绝来源未知/错误架构/不同 Web 或 eBPF 对象。
- 安装器先校验整个包，再暂存并实际执行两个二进制，保留既有配置。Mac、Debian 和 Alpine 均通过；损坏重装未改变任一已安装二进制。Alpine 实测发现的 `--check` BusyBox 兼容问题已修为 `-c`，没有隐藏该失败。
- 安装后真实 HTTP、CONNECT、SOCKS5 请求通过本地供应商到达本地目标；CLI/API 无帐号/cookie/令牌读写。Mac 用 T16 保留的不同 hpsd 写入 r2 和任务，停机快照后升级至候选，再回退旧二进制，配置/任务仍在。Linux 本轮演练相同二进制重启/停机快照恢复，明确不称为跨版本升级。新 schema 999 被拒绝，未强改版本号。
- 实际连接额度 4，第 5 个连接被拒，真实计数增加，已有连接关闭后额度归零，管理服务可用。占用管理端口启动失败后可再次启动，不遗留数据库锁或代理资源。

受控本地负载：16 worker、256 请求、约 3.44 MB 响应；Mac 0.466 秒、p50 28.54 ms、p95 33.58 ms；Debian 0.770 秒、p50 48.27 ms、p95 57.83 ms，均 0 失败，结束后活跃连接 0。这是同时运行其他编译任务时的单次本地夹具观察，不能比较平台性能或推断公网吞吐。首次夹具的默认 TCP backlog 5 无法承受并发、产生 502；扩大夹具 backlog 至 256 后通过，未修改产品来掩盖夹具容量问题。

## 真实 Linux 门槛

新的 CI eBPF 构建脚本实际执行：固定 nightly、经 SHA256 校验的官方 bpf-linker，重新编译 ABI4 对象得到 `fa5449609f86a6b50300a83a6e10d2f2150aa42eaa6676c8e64dd1581bbd5286`，与原 **48+4** 个真实案例的对象完全一致。历史完整工作区构建 ABI3 仅供拒绝测试；初版历史源码只提取子目录缺少 workspace 元数据的问题也已修正并实际重跑。

新聚合器读取真实 52 项串口/报告通过，故意缺一项时拒绝。发行包中的静态 hpsd 另外在 **6.12.115、7.2.14** 客体各实际执行一次独立进程 SIGKILL/重启、真实代理、外部规则保护及清理测试。两次客体内的 hpsd SHA256 都是安装包二进制 `1a7c24ef692345f42f915f96f357f28243c414be50cbe9532988880d49ac19a9`。见 [附加报告](release-kernel-2026-11-02/aarch64-6.12.115.json)、[另一内核](release-kernel-2026-11-02/aarch64-7.2.14.json) 及各自串口。单例 scope 的 `full_transparent_datapath_ready=false` 保留原意，不用一个案例替代完整 52 项结论。

## 需求与限制

[P0 映射](p0-evidence.json) 对 80 个唯一编号逐一归属到 Issue/实现/回归/证据；`scripts/verify-p0-evidence.py` 校验缺失与重复，但不会把“存在文件”当作测试通过。**没有宣称全部平台 P0 无条件通过**：SYS-05 的真实 macOS 系统代理持久写入未验证，旧的危险 ignored 测试已作废；开发宿主任何网络写入始终禁止。真实 launchd/systemd 生命周期、SDK 只读与无持久写入的事务/恢复检查是有效证据。

按用户授权，快代理官方契约和故障测试加免费源实测构成供应商证据。本轮免费页实际提取 15 条，仅获取/解析，没有拨打公开代理或推断健康；付费帐号、配额/白名单实际调用仍未验证。主动 UDP 健康探测当前明确 unsupported，严格持久崩溃 kill switch 未实现。以上边界不会在发行清单中改成绿色通过。
