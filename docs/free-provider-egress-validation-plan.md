# 免费供应商真实代理与出口验收计划（#29）

- 分支：`feature/free-provider-egress-validation`，从 `develop@c360880` 用 Git Flow start 创建。
- 免费来源：用户此前提供的 `http://www.ip3366.net/free/?stype=1`，当前页面 200 并提供 15 个端点。
- 国内目标：`https://myip.ipip.net/`（中文 IP 回显，TLS 默认验证）；`http://myip.ipip.net/` 用于补充 HTTP absolute-form 验证。`4.ipw.cn` 在当前宿主解析失败，已在沙箱外重试，不依赖它完成验收。

## 验收链路

1. 用禁用 `.curlrc`、显式不使用代理的 curl 建立 IPv4 直连基线；原始出口仅存本机临时记录，不提交本机地址。
2. 下载公开页面，独立核对 IP/端口，与实际 ProviderClient 发布租约比对；不提交第三方完整 HTML。
3. 少量并发逐个使用 `curl --proxy http://IP:PORT --noproxy ''`，分别测试 HTTP 和 HTTPS；记录 curl 退出码、真实 TCP peer、CONNECT 状态、TLS 校验和返回内容。仅 HTTP 200 或 curl 退出 0 不算代理成功；要求返回有效出口，HTTPS 还须验证目标 TLS。
4. 启动独立 `--ephemeral` Daemon，管理/HTTP 入口使用 127.0.0.1 的专用端口，来源、健康策略、租约均为临时测试配置；实际调用提取接口及健康检测。
5. 用 `curl --proxy http://127.0.0.1:<测试端口>` 验证 hps 选点与真实出口，关联节点/租约和健康结果；池全部不健康时应拒绝请求，不允许静默直连。
6. 若直接 curl 成功而 hps 同端点/目标失败，追踪应用代码并修复，补充覆盖实际缺陷的回归，再做相同真实端点验证。若直接 curl 也失败，不把供应商死节点或当前网络可达性问题伪造为软件缺陷。
7. 保存脱敏结果、必要修复和复现脚本，分批提交；回复真实结果到 #29 后关闭，不主动 push/finish。

## 代码与运行边界

ProviderClient 默认不继承环境代理、不重定向、不自动重试；YunFree 解析 GBK HTML，HTTP/HTTPS 表格标签均视为 HTTP/CONNECT 端点，TTL 仅为本地缓存上限。健康默认启用，新租约须实际穿透后才能准入。重点检查 `crates/provider`、`crates/health`、`crates/engine` 与 `crates/application` 的真实链路。

所有开发/测试进程设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`。不修改宿主 Wi-Fi/DNS/路由/系统代理，不安装或重启原服务，不运行旧网络写入测试，不覆盖原 Desktop bundle、持久数据库或原始未跟踪文件。网络请求无账号、Cookie、采购或用户业务凭据。
