# T16 Desktop 连接档案与系统服务实施计划

对应 #21，依赖已关闭的 #7/#10/#20。沿用相同 React/Tailwind MPA 与原生 HTTP/SSE transport，不新增项目登录、帐号或权限。

1. 连接档案：独立 Rust 本机集成库，CLI/Desktop 共用有界、版本化、加锁和原子落盘的本机/远端档案；端点规范化与 CA 路径校验。保留旧 Desktop 地址，显式选择后取消旧订阅并整页切换，旧请求不能污染新目标。远端地址即使是端口转发的 loopback，也保持用户选择的远端语义。
2. 持久服务：CLI 的显式本机命令与 Desktop 原生桥共用 OS adapter。macOS launchd 用户服务及 Linux systemd 用户/系统服务分别建模；Linux eBPF 通过管理员安装的系统服务运行，绝不提升 WebView。安装到持久位置，真实管理器状态与 Daemon 就绪分别展示；精确校验自有 unit/manifest，拒绝覆盖外部服务。停止/卸载保留配置与数据，窗口退出不停止服务。
3. 系统代理：仅明确的本机档案和对应已运行入口可修改当前客户端的代理；使用 OS 原生持久设置，修改前持久保存完整原值。恢复前检查仍为本应用写入值，出现外部更改时保留备份并报告冲突，不能覆盖后来改动。未支持的平台或 OS 拒绝必须明确展示。此检查属于对象归属与可执行性，不是项目用户鉴权。
4. 共用页面：连接页管理档案与本机服务引导；设置页展示本机服务/系统代理。浏览器继续管理同一套远端功能，OS 操作引导使用 CLI。原生动作带明确目标快照，切换后拒绝旧动作。
5. 验证：聚焦 Rust 并发/损坏档案/foreign unit/代理恢复冲突，CLI 无桌面闭环，实际隔离 launchd 服务安装、启动、窗口退出仍提供真实代理；独立 bundle 标识与 target 目录检查 MPA 导航/原生 transport/目标切换。Linux 用独立 systemd 环境验证，已有透明内核矩阵复用。全部证据真实记录后回复 Issue、关闭并按顺序 finish。

保护原来的 17900 服务、原 Desktop bundle 与偏好及用户原始未跟踪文件。验收仅使用独立端口、独立服务 label/state 目录与独立 bundle；不修改当前系统代理作日常测试，采用独立 OS 网络配置或隔离测试会话验证恢复。开发分批提交，不 push 或合入 main。

服务生命周期参考 [systemd.service](https://freedesktop.org/software/systemd/man/latest/systemd.service.html) 与 [Apple launchd](https://support.apple.com/en-asia/guide/terminal/apdc6c1077b-5d5d-4d35-9c19-60f2397b2369/mac)。严格区分启动命令成功、进程存活和实际 API/代理 ready。
