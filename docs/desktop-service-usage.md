# Desktop 连接档案与本机服务

Desktop 与浏览器使用相同的 React/Tailwind 多页面产物。Desktop 原生桥负责本机连接选项、证书、系统服务和本机 OS 操作；服务器始终可以只安装 `hps`/`hpsd`，不依赖窗口、WebView 或 Node.js。

## 从 Desktop 启动本地服务

在「连接服务」或「系统设置」找到「Desktop 本地服务」，点击「启动本地服务」。Desktop 在自身进程中运行代理引擎和管理 API，并在启动后打开运行概览。这个模式适合希望随 Desktop 启停的本机代理。

- 管理 API 由系统在 `127.0.0.1` 分配端口。首次启动准备 HTTP 和 SOCKS5 回环入口，实际地址见「代理入口」。配置可用出口和路由后，代理数据面才会就绪。
- 选择「停止本地服务」关闭这个实例。如果正在使用它，Desktop 会回到保存的连接。退出应用也会停止服务，关闭到托盘时服务继续运行。
- 数据位于客户端状态目录的 `desktop-service` 子目录，开发构建使用 `desktop-service-dev`。配置、草稿和数据库在重启后保留。同一数据目录同时只允许一个实例启动。
- 独立运行的 Daemon 和已安装的系统服务可通过连接档案管理。系统代理写入入口只随显式本机档案提供。

## 连接档案

在 Desktop 的“连接服务”页创建档案。`local` 档案只允许 loopback 地址，明确表示用户正在管理本机；`remote` 档案即使使用 localhost 端口转发，也不会打开本机 OS 控制。档案不包含项目帐号或登录信息。可为 HTTPS 配置 PEM CA 文件的绝对路径，原生 HTTP 和 SSE 共用相同证书。

CLI 与 Desktop 默认共用本机档案目录：macOS 为 `~/Library/Application Support/honk-proxy-server`，Linux 为 `$XDG_CONFIG_HOME/honk-proxy-server` 或 `~/.config/honk-proxy-server`。`HPS_CLIENT_STATE_DIR` 可显式指定独立的绝对路径。档案至多 32 个，修改带 revision、跨进程文件锁与原子落盘；冲突后必须重新读取，不能隐藏覆盖其他编辑。

```sh
hps --output json profile list
hps profile save local --name "本机服务" --local --address http://127.0.0.1:17900 --expected-revision 0
hps profile select local --expected-revision 1
hps --profile local status
```

示例 revision 只适用于空目录；应先读取实际 revision。显式 `--endpoint` 优先选择临时连接，并与 `--profile` 互斥；未给端点/档案时使用已选档案，否则使用默认 loopback 地址。切换 Desktop 档案后重载页面，取消旧订阅。原生 JSON、SSE、文件导出和本机动作均绑定目标及页面代际；旧页面排队中的请求不会借切换后的地址执行。

## 独立系统服务

安装本机服务前准备经过校验的配置，保证管理监听与档案的 loopback 端点、端口和 TLS 匹配。Desktop 在连接页提供独立程序/配置路径和安装、启动、停止、重启、卸载入口。CLI 的服务命令必须明确 `--local`；它不会发送到远端 API，也不接受全局 `--endpoint`/`--profile`。

```sh
hps service --local --id local install --binary /opt/hps/bin/hpsd --config /etc/hps/hps.toml --address http://127.0.0.1:17900/
hps service --local --id local start --expected-endpoint http://127.0.0.1:17900/
hps service --local --id local status
hps --endpoint http://127.0.0.1:17900 status
hps service --local --id local stop --expected-endpoint http://127.0.0.1:17900/
```

- macOS 使用自有 label 的 launchd 用户服务；Linux 使用 systemd 用户服务，管理员 CLI 的 `install --system` 可安装系统服务。Linux eBPF 配置需要管理员安装的系统范围服务；Desktop/WebView 拒绝 root 运行，不自动提权整个应用。
- 安装将程序和解析过相对路径的配置复制到持久目录，数据库独立保存。Linux 系统服务通常由管理员使用独立的 `HPS_CLIENT_STATE_DIR` 管理；Desktop 可通过本机档案连接其 API，不将管理员文件操作伪装成普通用户能力。
- 操作前验证 manifest、unit 内容、服务 label 和 expected endpoint；外部 unit 或路径冲突会拒绝接管。启动命令成功、OS 进程存活、管理 API 就绪和代理入口就绪分别显示。
- 关闭窗口或退出 Desktop 不停止系统服务；停止只作用于明确的服务。卸载停止并移除自有 OS 注册，保留配置、程序和数据库。同一 scope/endpoint 可重新安装并继续使用已有数据。
- 安装后仍需显式启动。launchd/systemd 负责异常退出重启；不使用窗口临时子进程承诺持久服务。Desktop 自启与 Daemon 开机选项在 T20 分别管理。

## 系统代理与开发限制

macOS 的设置面板提供网络服务、当前实际运行的 IPv4 HTTP/SOCKS5 入口和原值恢复。HTTP 入口可供 HTTP 请求及 HTTPS CONNECT 共用，不把 TLS 到代理的协议混同为系统界面的目标类型。选项来自实际监听，远端档案不能修改本机。

修改前原子保存完整原值，包含原 PAC、自动发现、bypass 和其他代理字段。只修改已有 UUID 服务的 `Proxies` 子路径，不替换 NetworkServices、DNS、IPv4 或接口。恢复必须仍匹配本应用写入值；外部变化时拒绝覆盖并保留备份。提交/应用失败同样保留恢复记录。操作系统可能要求自身的管理员授权，这不引入项目认证。

**开发宿主 Mac 禁止网络写入验收。** Cargo 环境强制 `HPS_FORBID_HOST_NETWORK_CHANGES=1`，启动独立验收应用也必须带该变量；普通测试拒绝可写会话和持久提交。修复后的验证覆盖事务恢复、只读 OS 服务、真实 SDK 的私有会话内存暂存及文件未变，不能声称实际活动网络修改已经验收。事故及作废证据见 [事故记录](validation/macos-network-incident-2026-11-02.md)。实际持久 OS 修改仅可在确认隔离的系统中进一步验证，不能重新以宿主测试的方式验证。

没有系统代理后端的平台明确显示不支持，仍可使用浏览器 HTTP/HTTPS/SOCKS5 配置；Linux 服务端和 eBPF 均不依赖客户端桌面系统代理。所有网络配置真实副本仅留本机私有目录，禁止提交或随诊断导出。
