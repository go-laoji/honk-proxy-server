# 并行 Backlog 最终集成与发行候选

本记录对应 2026-11-02 的 T18–T21 集成。功能源码固定为 `862ddad624403e6f81bc994a7fe262ac04b7fb01`，共享 Web build ID 为 `7c94170ee16c7b9c2f8e0aa08c219e172c2a7b6bb4834c71b2cf96a2fb4316e1`，18 个独立 HTML 入口。后续提交只记录文档、验收和 Git 收尾，不把文档 HEAD 重新标成已编译的源码版本。

用户最终要求当前开发以 macOS 验收为准，停止 x86 静态构建并清理专属环境。以下已完成的 Linux 结果保留为历史证据；取消的 x86 发行构建/安装验收不算通过。

## 交付范围

- T18 / #23：有界历史趋势、规则命中、健康历史、DNS64 映射视图、规则版本导入导出、诊断任务、取消/重试与告警。真实采集完成和持久任务终态分离，持久化失败不提前回收控制项。见 [使用说明](../observability-usage.md)。
- T19 / #24：SOCKS5 UDP ASSOCIATE；HTTP/HTTPS/SOCKS5 的 URI/base64/Clash 订阅子集；独立配置代际的订阅快照；有界嵌套组和可解释 Score。短效 API 租约与订阅不混用，来源最小寿命取当前代际策略。YAML anchors/aliases 在展开前拒绝。见 [使用说明](../protocol-subscriptions-usage.md)。
- T20 / #25：SQLite Backup API 一致性快照、单份在途下载预算、严格 schema/内部内容校验、封存输入、生命周期维护锁和离线原子恢复；Desktop 托盘/告警通知/登录自启适配；Linux 拓扑与自有 eBPF 对象只读诊断。见 [使用说明](../backup-desktop-diagnostics-usage.md)。
- T21 / #26：已完成四项储备能力的产品、平台和兼容契约。多实例控制器、Mac/Windows 透明捕获、真实成本对账、外部规则更新及目标 FakeIP 未作为当前运行功能交付，见 [范围契约](../future-platform-contracts.md)。

跨分支发现、修复来源和只读复核边界见 [审查记录](parallel-review-2026-11-02.md)。未引入 honk 运行依赖、Go 后端、项目登录、帐号或权限模型。

## 组合验证

| 环境 | 实际结果与边界 |
| --- | --- |
| macOS aarch64 | 最终源码 workspace 227、Native 11 通过；all-target Clippy、fmt、TypeScript、Prettier、18 页构建通过 |
| Chromium | 完整组合 25 项基于 `695e116`；随后存储修复没有更改 Web 字节；最终 `862ddad` 的持久备份下载/摘要、Linux 页面、移动设置 3 项通过；不把两轮相加 |
| Linux aarch64 | workspace 234 通过、30 ignored；ignored 不算实际内核验收；all-target Clippy 通过 |
| Linux x86_64 | qemu-user workspace 233 通过、30 ignored、1 项明确排除；原 socket 选项案例在用户态模拟器 errno 92 失败，同一未剥离 ELF 在两个完整 x86 内核中各通过一次；保留原失败，不把它记成普通测试通过 |
| Linux 实际内核 | aarch64/x86_64 × 6.12.115/7.2.14，标准案例共 52；其中新诊断断言涵盖 12 组自有对象 ID/独立计数及控制状态不变的证据 |

标准矩阵聚合只计算上述 52 项：[acceptance.json](backlog-kernel-2026-11-02/acceptance.json)。ABI 4 eBPF 对象 SHA-256 为 `fa5449609f86a6b50300a83a6e10d2f2150aa42eaa6676c8e64dd1581bbd5286`。额外 socket 补验和最终静态 release Daemon 案例不加入 52 的计数。最低实际验证内核为 6.12.115，不推断更早版本可用。

最终安装运行验收包含 18 页、无帐号 CLI/API、HTTP/CONNECT/SOCKS5、256 请求/16 worker 自有回环负载、配额耗尽后管理接口可用、连接容量回收、管理端口占用拒绝和恢复。吞吐只是本地 fixture，不代表公网性能。CLI 备份验证下载摘要、运行中恢复拒绝、停机恢复/原件保留、错误摘要拒绝和重启。

Mac 额外使用实际旧版 T17 二进制（摘要 `9e90c470b2412d16df14690fa0e293d702bec05475f85566d0828497309a78ae`）完成跨源码升级和停机快照整体回滚；Linux 当前的同二进制重启不记作跨版本升级。Mac 组合恢复还验证订阅快照/稳定节点 ID/未知到期时间、脱敏视图、告警设置、持久诊断结果和新 epoch。详见 [Mac 验收](backlog-macos-2026-11-02.md)。

## 发行候选与限制

| 候选 | 主工作区本地文件 | SHA-256 | 已执行结果 |
| --- | --- | --- | --- |
| macOS headless | `target/validation/backlog-candidate/macos-862ddad/hps-headless-0.1.0-aarch64-apple-darwin.tar.gz` | `573959469df074c9f11f650ce9ed61a8849a2878a44d01bebe29de6d25b7adb1` | 安装、协议、故障、备份、跨源码升级/回滚通过 |
| macOS Desktop | `target/validation/backlog-candidate/macos-desktop-862ddad/hps-desktop-0.1.0-aarch64-apple-darwin.zip` | `723fca2a24927e0247425e9157fca554e492c5f1f454301c18fda478f67d5818` | 签名/归档/元数据通过；可见窗口及OS行为未实测 |
| Linux aarch64 static | `target/validation/backlog-candidate/linux-aarch64/hps-headless-0.1.0-aarch64-unknown-linux-musl.tar.gz` | `74e0dc1bd1355e6f6693e67d7b3d2830968f01770eb82778cebde07ba6582906` | 安装、故障、备份及两个内核的release程序案例通过 |
| Linux x86_64 static | 按用户要求取消 | 未生成 | 编译在最终优化中主动停止；未完成构建、安装和 release 内核验收，不算通过 |

本地文件保存在主工作区 `target/validation/backlog-candidate/`；安装报告、manifest 和普通构建原始日志保存在主工作区，内核 JSON 与其引用串口日志纳入 Git。Desktop zip 是 ad-hoc 签名候选，已验证签名、归档和相同 Daemon/Web；未发布商店、registry 或远端 Release。

Mac 原生可见窗口、托盘展示、OS 通知及真实登录自启仍未实测；计算机工具拒绝控制 Terminal，直接打开独立只读验收 app 又报告锁屏，因此没有以 UI 单元测试代替 OS 行为。Mac 活动网络写入被禁止，旧错误的写入测试结论作废。Windows 和 Linux Desktop 安装包、商店签名/公证、Linux deb/rpm 及真实付费供应商账户调用未验证。快代理按用户授权的官方契约/故障 fixture 验收，免费来源历史实际获取不等于持续节点可用性承诺。

## Git 与宿主边界

已依次完成 #24 `d3123be`、#25 `92e95a6`、#26 `4a45a58`，主工作区回到 develop。累计 22 项 feature finish 的第二父原 tip、双父、内容树一致及 develop 可达性逐条通过。见 [完整审计](git-flow-finishes-2026-11-02.json) 与 [收尾流程](../git-flow-workflow.md)。开发子 Issue #6–#26 均已回复实际结果并关闭；本轮以用户最后指定的 macOS 开发验收范围收尾，取消的 x86 额外发行验证不计为通过；未来正式 Linux x86 发行前须重建环境并完成该包验收。main 保持 `18d589913b37ef4caa98c2c0a89f32ebfb38a3a9`；未 push、删除远端分支、合入 main 或移动旧 RC 标签。

开发宿主 Mac 未进行新的 Wi-Fi、DNS、路由或系统代理配置写入；验收网络操作只在自有 Linux 容器/完整虚拟机中进行。主工作区临时只读验收 Daemon 已停止；不恢复旧网络配置、不启动用户原服务、不覆盖原 Desktop bundle。无关原始未跟踪文件保持在主工作区。

## x86 开发环境清理

已停止并自动移除编译容器，删除两个内核保留容器、四个项目 x86 镜像标签和两个专属 cargo/rustup 卷；仅移除共享 target 中 `t17-static-x86`、`x86_64`、`x86-userland`、`x86-stripped` 四个目录，共 18.97 GiB。ARM 产物/缓存、ABI4/ABI3 对象、共享 Rust 基础标签、binfmt 和其他项目资源保留，没有全局 prune。详见 [清理记录](x86-development-cleanup-2026-11-02.json)。取消时 partial build 日志及已提交的真实内核证据保留；未生成 x86 最终包。
