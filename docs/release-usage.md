# 分包构建与安装

源码、依赖和工具链见 `packaging/toolchains.json`。先提交相关改动，使用锁文件安装并执行一次 `pnpm build`；把所得 `dist` 作为两个包的共同输入，构建期间不改动它。`hps build-info`、`hpsd --build-info` 和 Desktop 的单独 `--build-info` 参数只输出编译元数据，不读取配置、启动窗口或连接服务。

```sh
pnpm install --frozen-lockfile
pnpm build
scripts/build-headless.sh
python3 scripts/package-headless.py --bin-dir target/release --target aarch64-apple-darwin
python3 scripts/package-desktop.py --daemon target/release/hpsd
```

命令中的 target 必须匹配实际执行平台。Linux 便携包使用原生架构 Alpine builder，并显式设置 `RUSTFLAGS='-C target-feature=+crt-static'`、`--target <arch>-unknown-linux-musl`。打包器检查 ELF 架构、静态链接、实际可执行的源码元数据及 Web 构建 ID。默认不接受旧源码、未知来源或未提交源码。`--allow-dirty` 只生成明确标记的试验包，不能作为正式候选；`--debug` Desktop 也必须带此标记。

仅修正文档或安装/打包脚本后，headless 可显式传 `--source-revision <完整40位已编译commit>`。它检查 Rust、锁文件、构建配置、Desktop 和前端输入与当前 HEAD 完全一致，再分别记录真实编译源码与打包源码；任何编译输入变化都必须重建，不能用此参数给旧二进制换标签。

Linux 两类安装包还必须传入 `--ebpf /absolute/hps-ebpf.elf --kernel-report /absolute/acceptance.json`。报告必须来自真实的 52 项双架构/双内核案例，并与对象 SHA256 一致；只编译对象、预检或单个内核通过都不够。旧 ABI 对象仅是拒绝测试夹具，不进入发行包。最低实测内核为 6.12.115，ABI 4。

Headless 包含 hps/hpsd、默认配置、用法文档、清单和校验和；Linux 另外包含 eBPF 对象和 systemd 示例。解压后先核对外部 `.sha256`，运行目录内的安装脚本再次核对每个文件：

```sh
./install.sh --prefix /absolute/install-prefix --config-dir /absolute/config-directory
```

安装器不启动服务、不改 OS 网络，保留已有配置，新配置权限为 0600。运行不需要 Node、Python、WebView 或桌面显示。随后通过 `hps service --local ...` 显式安装和启动受操作系统管理的服务，参数见 Desktop/服务用法。不要把打包解压路径中的临时二进制当作持久服务路径。

Desktop 打包默认写入独立的 `target/desktop-release`，拒绝覆盖原始 `src-tauri/target`。资源中含相同源码、相同 Web 的 hpsd 和默认配置；Desktop 安装服务时把资源复制到持久目录。Mac 候选只做本地 ad-hoc 签名，未云签名/公证；Linux 当前输出 deb。不要将未执行的平台流水线称为已验收的平台。

## 升级与回退

先停止本实例服务，保存配置和整个数据目录的停机快照，然后安装新包并启动，核查 ready 状态、运行版本、节点/任务和真实代理请求。安装器依次替换两个文件，不承诺两个文件在断电时作为一个事务切换，所以必须在停机后升级。配置不会由安装器覆盖。

二进制回退前停止新服务。相同 schema 且旧版本支持时可保留数据库；发生 schema 升级时必须恢复升级前的完整停机快照，再启动旧版本。不复制写入中的 SQLite 主文件当作备份，不修改 schema 版本号强迫旧程序读取。在线备份是独立的 CFG-07 功能。

## 工作流与证据范围

日常 Windows/Linux/macOS 构建使用独立的 `build.yml`，支持 push、PR、标签和手动执行，产物及下载说明见 [三系统 CI](ci-builds-usage.md)。它生成明确标记的普通 CI 包，不包含 Linux eBPF 对象、不替代以下发行验收门槛。

产品版本由 cargo-release 统一管理，不发布 registry、不自动 push；版本 tag 推送后，`build.yml` 的全部原生构建成功才会自动发布对应 GitHub Release 附件，见 [版本与 Release 使用说明](version-management-usage.md)。GitHub Release 的通用构建附件与下述完整内核验收包保持各自的真实能力记录。

`.github/workflows/release.yml` 只在 PR 或手动启动时运行，没有发布、push、远端删分支、自动启用服务或系统网络操作。它只构建一次 Web，真实内核工作并行跑两种架构的两个客体内核，全部 48+4 案例通过后才把对象与证据传给独立 headless/Desktop 工作流。Actions 和 Rust 基础镜像固定摘要，官方 bpf-linker 0.12.0 下载需先核对固定 SHA256。

工作流已加入源码；没有获授权推送，远端 CI 尚未实际执行。本地验证结果独立记录在 `docs/validation`。GitHub runner 标签参考 [官方 runner 清单](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)，Linux Desktop 依赖参考 [Tauri 官方前置条件](https://tauri.app/start/prerequisites/)。

宿主 Mac 始终设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`；只能读配置和无 OS 写入的事务/恢复验证。真实持久代理修改明确未验收，事故前的旧测试无效。付费供应商帐号调用也未验收；按用户授权，快代理官方契约/故障夹具和免费源实测分别列出。
