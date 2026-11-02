# 余下 Issue 组合验收：Mac 与本地候选

最终运行代码为 `862ddad624403e6f81bc994a7fe262ac04b7fb01`，包含 #23、#24 与 #25。详见[机器摘要](backlog-macos-2026-11-02.json)及[备份与桌面使用说明](../backup-desktop-diagnostics-usage.md)。本页不替代 Linux 真实内核矩阵，也不把 Mac 可见窗口、OS 通知投递或登录自启算作通过。

- 最终源码：Mac workspace 227 passed、Native 11 passed，均 0 failed/ignored；整仓与 Native all-targets Clippy `-D warnings`、fmt、TS、Prettier、18 页生产构建通过。
- 组合源码 `695e116` 的共享控制台全量 25 项通过；随后备份完整性修复到 `862ddad`，最终持久库浏览器 3 项通过，覆盖实际 SQLite 下载/摘要、只读诊断展示及移动布局。前端字节未变，Web ID `7c94170ee16c7b9c2f8e0aa08c219e172c2a7b6bb4834c71b2cf96a2fb4316e1`。
- 最终 headless 包的实际安装、摘要与已有配置保留、损坏升级拒绝通过。实际安装二进制完成 HTTP、CONNECT、SOCKS5、256 请求/16 workers 本地负载零失败、连接归零、4 连接限额/第五条拒绝、管理接口可用、端口冲突与干净重启。
- 跨源码版本实测：保留的 T17 二进制 `2b196752...` 创建配置/任务，最终 `862ddad` 升级读取并完成真实代理；停机后恢复完整旧数据库快照，再由旧二进制正常启动。未来 schema 999 被明确拒绝，没有修改版本号强制读取。普通同二进制重启/回退与此跨源码用例分别记录，性能仅为本地夹具，不承诺公网吞吐。
- 安装后的独立 CLI/Daemon 实测在线备份、运行锁拒绝、离线恢复保留原库、坏摘要拒绝和重新启动；标准订阅 CLI 的七项检查通过。
- 组合恢复另建自有实例：保存含凭据的独立订阅快照、关闭告警并持久诊断任务，备份后删除来源，再离线恢复。重启后来源/稳定节点 ID/未知到期、脱敏视图、告警设置及诊断结果都保留，运行 epoch 更新。未抓取示例订阅 URL，未调用付费接口。

Mac headless 归档 SHA-256 为 `573959469df074c9f11f650ce9ed61a8849a2878a44d01bebe29de6d25b7adb1`；Desktop zip 为 `723fca2a24927e0247425e9157fca554e492c5f1f454301c18fda478f67d5818`。两者编译来源均为 `862ddad`、dirty=false、18 个共享页面同一 Web ID，Desktop 资源中的 hpsd 与 headless 二进制 SHA `6edcb1f19c7449714c7bd63ecab54bfc7c8e56468a4e1313a8c45716b730eccb` 相同。Desktop 本地 ad-hoc 签名、deep/strict 验证及 zip CRC 通过；未云签名/公证。

候选位于主工作目录 `target/validation/backlog-candidate/macos-862ddad` 和 `macos-desktop-862ddad`，Desktop 采用独立 `target/validation/backlog-desktop-release`，没有覆盖原始 `src-tauri/target` bundle。已有 T17 候选保留，旧 `695e116` 暂存候选不作为最终交付。

所有实例/数据目录/端口都为自有临时对象，已清理自有运行进程；未启停用户原服务。Desktop 仅执行离线 build-info 和文件签名检查，可见窗口验收时工具报告 Mac 锁屏，未运行 GUI、未变更登录项或 OS 通知偏好。宿主 Wi-Fi/DNS/路由/系统代理没有写入；实际持久 Mac 代理修改与付费供应商帐号仍保持未验收。没有推送、公开发布或修改 main。
