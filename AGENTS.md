# 本项目开发与验收边界

- 后端使用 Rust，CLI/Daemon 独立于 Desktop；前端共用 React/Tailwind 多页面产物；不引入项目登录、帐号或权限验证。
- Git Flow 按用户约定 start 分支并小步提交。用户已明确授权：验收通过的分支按依赖顺序逐个 finish，保留独立合并记录，不跳过较早分支，不主动 push 或合入 main。GitHub 开发任务须回复真实结果后再关闭。
- **不得修改开发宿主 Mac 的活动网络配置，也不得使其 Wi-Fi、DNS、路由或代理断网。** 2026-11-02 的代理写入测试曾误写默认配置，旧的 `real_systemconfiguration_private_preferences_commit_and_restore` 验收结论无效，禁止重新运行旧测试二进制。
- macOS 宿主验收只允许网络配置只读检查及无 OS 写入的事务/恢复测试；`cargo test` 必须拒绝创建可写系统配置会话。开发应用必须设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`。需要实际网络配置写入的验收只能在确认隔离的虚拟机内进行，不能把临时文件路径、mock 或 ignored 当作隔离证明。
- Linux 网络与真实 eBPF 验收使用自有容器/虚拟机、独立 cgroup/netns 和明确的对象归属，不修改宿主 Mac 网络。保护用户原服务、Desktop bundle、偏好和原始未跟踪文件。
- 操作系统网络配置和恢复副本含本机隐私，不得提交仓库或发送到 Issue。发现事故先停止写入、只读诊断并保留副本；不得自动恢复旧网络配置而再次影响连接。
