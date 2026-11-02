# T20 / #25 实施计划

## 边界与契约

- CFG-07：在线备份使用 SQLite Backup API，独立只读连接、有界逐页复制，最终 quick_check、schema 与 SHA-256 校验。私有临时文件不完整时删除；备份默认含凭据，必须明确提示。普通诊断包继续只返回白名单元数据。
- 恢复仅允许本地离线 CLI：复用 Store 的 daemon.lock 独占锁，持锁期间校验备份并创建一致候选，保留原库与 WAL/SHM 后原子替换；拒绝在线恢复、新 schema、损坏文件和错误摘要，不覆写已有输出。重启后正常迁移与 epoch 清理，不能伪装热恢复。
- SYS-06：Desktop 专属托盘显示/退出与关闭到托盘选项；通知沿用 Daemon 告警事实源，仅做 OS 投递与本地去重；Desktop 登录自启与 Daemon service 启动完全分离，默认关闭。宿主验收不启用真实登录项或更改网络。
- LINUX-07：只读采集网络命名空间、接口/路由、cgroup/bpffs 拓扑，与当前运行实例已持有的 programs/maps/counters 和 revision 关系。变化比较只在同 epoch 内，reset 与 counter decrease 明示。无新数据面控制。

## 分片与验证

1. Storage Backup API、离线 restore、WAL 活动写入/损坏/版本/锁/权限测试；Rust API/typed client/CLI 及真实 HTTP 下载验收。
2. Linux 只读 topology API/typed client/CLI/共享 MPA，使用真实 Linux 自有隔离环境采集；不把 fixture 当内核验收。
3. Desktop 托盘/通知/登录自启 Native 桥与设置页面；编译、纯配置测试和独立标识包验收。锁屏时保留 UI 未验收说明。
4. 更新使用文档、验收证据、任务进展；各片 scoped commit。根代理负责依赖顺序集成，当前分支不 finish/push/关闭 issue。

## 安全条件

所有运行 Desktop 使用 HPS_FORBID_HOST_NETWORK_CHANGES=1。不运行旧 SystemConfiguration ignored 测试或旧二进制，不更改宿主 Wi-Fi/DNS/路由/代理，不动用户原服务、bundle 或偏好。恢复验收仅使用临时测试数据库，系统配置和恢复副本不提交。
