# T16 Desktop 档案与服务验收

对应 #21、INST-03、SYS-05；机器记录见 `desktop-service-2026-11-02.json`，使用方式见 `../desktop-service-usage.md`。

## 实现与验证

- Rust `hps-system` 共用档案、文件锁/CAS、launchd/systemd 自有服务、代理原值事务。CLI 新增 profile/service；本机命令明确 --local，远端不能借 localhost 地址误入本机动作。
- Desktop 仍使用同一套 18 页 React/Tailwind MPA。JSON/SSE/本机动作绑定目标及页面代际；旧请求不能在切换后的地址执行。CA 同时用于原生 JSON 和流订阅。WebView 拒绝 root；Linux eBPF 使用管理员安装的系统服务。
- macOS 系统库 9 项、原生桥 9 项、Linux 系统库 5 项、完整 Chromium 18 项及相关 clippy 通过。新增 Web 产物完整性检查属于后续 T17，不计入本轮数量。
- `tests/cli/service.py` 在真实 macOS launchd 用户服务与隔离 Linux systemd 257 系统服务分别通过：独立 HTTP 代理字节、错误端点拒绝、foreign unit 保留、SIGKILL 后 OS 自动重启、卸载重装保留 SQLite。未修改宿主网络配置。
- 此前实际窗口已验证档案创建/选择、localhost 远端禁用本机控制、原生 JSON 写入/读取、SSE 实时及本机服务启停。修复后的独立安全包再次验证设置页、原生读取和网络保护提示，两个代理修改/恢复按钮均禁用。
- 独立应用退出后自有 launchd PID 保持，管理/代理就绪且实际 HTTP 代理响应通过。测试只操作自有服务和 loopback 目标；不重启用户原来的应用或服务。

## 必须保留的边界

macOS 网络写入事故与修复见 `macos-network-incident-2026-11-02.md`。旧的所谓隔离写入 pass 已作废，未计入任何验收。用户明确禁止使宿主 Mac 断网，后续测试不再创建宿主可写网络会话；Cargo 与独立验收应用启用写入禁用标志。

系统代理原值、部分提交失败和外部更改冲突通过无 OS 写入事务验证；实际 SDK 只在自建文件的会话内存暂存/恢复已有服务的 Proxies，并验证文件、其他服务、DNS/IPv4 未变。该测试不等同于实际系统持久修改；修复后的活动网络提交/应用仍明确未验证，不能在发行说明中声称已通过，也不得在本机重跑旧测试。没有 OS 后端的平台明确不支持系统代理自动修改，Linux 服务端与真实 eBPF 不依赖该客户端功能。

#21 收尾依据是档案、共享页面、真实持久服务、窗口独立生命周期、目标隔离及上述明确的代理事务/SDK 验证。T17 对发行范围和证据继续单列，T20 再实现桌面自启/托盘等增强；不把本轮交付冒充这些后续功能完成。
