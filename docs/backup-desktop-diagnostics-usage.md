# SQLite 备份、Desktop 体验与 Linux 诊断（T20 / #25）

## 一致性备份与离线恢复

设置页的“数据库备份与恢复”要求确认备份含凭据，再生成完整 SQLite 文件。Daemon 的 `POST /api/v1/config/backup` 使用独立只读连接与 SQLite Backup API，固定 WAL 只读事务快照，每次最多 128 页、间隔 5ms、总期限 30s、文件上限 64MiB、每实例最多一个正在生成或下载的完整备份。不会复制正在写入的主文件，也不会把备份放进普通脱敏诊断包。

候选存储在数据目录中随机命名的私有临时文件，quick_check、foreign_key_check、声明版本对应的参考迁移 schema、所有配置历史的内容摘要、草稿/时钟/事件单例、持久租约/健康记录的可解析性及文件 SHA-256 都验证后才返回；失败清理候选。结果为独立 DELETE-journal SQLite 文件。`X-HPS-Backup` 包含格式、schema、大小、摘要与校验时间。响应 `Cache-Control: no-store`。下载响应持有容量许可直到消费完成或丢弃；分块输出避免尾部慢帧保留整个快照。Native 保存对话框也限制每进程一份等待快照。CLI/Native 下载另验大小与摘要，保存使用 0600 且拒绝覆盖已有文件。浏览器保存权限由浏览器/OS 决定；安全上下文可用 Web Crypto 时另验摘要，普通远程 HTTP 仍校验大小并显示服务器摘要供离线 inspect 校验。

```sh
hps --endpoint http://127.0.0.1:17900 --timeout 45 --output json backup create ./hps-backup.sqlite3
hps --output json backup inspect ./hps-backup.sqlite3 --sha256 <下载返回的摘要>
# 使用你已经配置的服务管理方式先停止目标 Daemon，再在目标本机运行：
hps --output json backup restore ./hps-backup.sqlite3 --offline --data-dir /private/hps-data --sha256 <摘要>
```

restore 不接受远端 endpoint/profile，不通过管理 API 触发，不会自动停止/启动任何服务。它要求已存在的数据目录，复用 Daemon 生命周期 `daemon.lock` 的独占锁；只要 Store 仍在运行，就会拒绝替换。先验证输入的大小、摘要、schema 和完整性，再用 Backup API 生成候选。输入必须是独立 DELETE-journal 快照，不能拿活动 WAL 主文件当备份。schema 高于当前支持版本会拒绝；受支持旧版本在下一次正常启动按既有迁移处理，不强改版本号。

持锁期间 checkpoint 原库，保留 `hps.pre-restore-<随机ID>.sqlite3` 硬链接，再原子替换主库并同步目录。替换前失败原库保持可用，候选清理；替换完成但目录同步失败会明确报告“已发布但持久性不能确认”，此时保持服务停止并检查保留原件。完成后 CLI 返回 `restart_required: true`，用户显式重启 Daemon。正常启动会生成新 epoch、标记旧进行中任务 interrupted、撤销旧健康证明，重新建立控制器。不要让旧 Store 或后台写入跨越恢复。原库保留文件也含凭据，需按备份妥善保管。

备份覆盖数据库中的配置/凭据/租约/任务/事件，不包含外部 TLS 证书、规则源文件、Desktop 偏好、服务管理定义或 OS 网络配置。移机时需自行提供仍被配置引用的外部文件。来源配置中的监听/上游地址会在恢复后生效；请先在合适的目标环境审查。

## Desktop 托盘、通知与登录自启

Desktop 使用 [Tauri 托盘](https://v2.tauri.app/learn/system-tray/)、[通知](https://v2.tauri.app/plugin/notification/)和 [Autostart](https://v2.tauri.app/plugin/autostart/) 适配。设置页的 Desktop 偏好分别提供：

- 关闭窗口保留在托盘，默认关闭；托盘不可用时拒绝开启，避免窗口丢失。托盘支持显示窗口与退出 Desktop。退出不停止独立 Daemon。
- OS 告警通知，默认关闭。仅轮询当前目标的 `/api/v1/observability/alerts`，沿用 Daemon 的告警/恢复和去抖事实，不自行产生第二份告警。
- 当前用户登录时启动 Desktop，单独查询实际 OS 状态；仅在用户保存偏好时切换。登录项名称为 `<app identifier>.desktop-login`。Daemon 的开机启动仍由本机 service 管理独立配置，两者互不启停。

通知首次连接、新 epoch、重新启用时以 `latest_sequence` 建立基线，跳过旧历史；后续按 `after` 最早分页逐条推进，以目标+epoch+sequence 持久去重。每条投递前先保留游标，避免进程崩溃后重复通知；因此投递失败/崩溃可能漏掉 OS 展示，完整事实始终以 Daemon feed 为准。保留缺口会显示错误状态。通知只在 Desktop 运行且选项开启时投递，系统勿扰/权限可能拦截展示。偏好页面保留最近投递错误，成功保存偏好清除该状态。

Desktop 偏好与游标写入本应用配置目录的 0600 `desktop.json`；不写用户原 service 配置，不控制系统代理。验收包使用独立 identifier，不能用生产 bundle/用户真实登录项做自动验收。

## 高级 Linux 只读诊断

`GET /api/v1/linux/diagnostics`、`hps linux diagnostics` 与 Linux MPA 使用同一 Rust 快照。内容包括当前 network/cgroup namespace、cgroup membership、内核版本、从 `/proc/net/dev` 确定的接口与 sysfs 属性、IPv4/IPv6 proc 路由、cgroup2/bpffs 挂载；以及当前实例已持有的 capture program/map IDs、kernel/userspace revision、flow/owner 数量与 hook 计数。

文件读取每项最多 128KiB，最多 128 个接口/挂载、256 个路由，超限会标明截断。它仅覆盖 Daemon 当前网络命名空间的 proc 路由表；不枚举其他 netns、复杂 policy rule/table，不承诺单个原子内核快照。不会运行 ip 命令、不进入命名空间、不加载/挂载新 BPF、不更改数据面。未知或不可读状态明确保留为空/警告。macOS/Windows 返回 unsupported，不伪装 Linux 验收通过。

```sh
hps --endpoint http://linux-host:17900 --output json linux diagnostics > before.json
hps --endpoint http://linux-host:17900 --output json linux diagnostics --baseline before.json
```

CLI 的 `topology_changed` 在同 epoch 内比较拓扑；`counter_delta` 只比较同 epoch、相同 program/map IDs、kernel revision、递增采样时间与单调计数，代次变化/计数回退返回 null。页面刷新同样明确代次重置和计数回退。已有计数证明观测到内核对象，不能代替真实转发/故障回收验收。T20 未新增数据面控制；严格持久崩溃 kill switch 仍属保留能力。
