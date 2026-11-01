# T04 显式代理实施计划

任务：[Issue #9](https://github.com/go-laoji/honk-proxy-server/issues/9)。依赖 T01 已完成；T03 的真实 Linux eBPF 单独保留，显式代理与后续透明入口共用目标上下文和拨号边界。后端完全 Rust，不集成 honk。

## 交付切片

1. 新增 `crates/engine` 与 TOML 入口/上游/限制配置。统一目标与出站拨号，支持 HTTP/HTTPS CONNECT、SOCKS5 及供应商凭据，默认验证 TLS；到期、禁用及剩余寿命不足的节点不准入。静态节点只用于核心验证，不替代短效供应商 API。
2. 实现 HTTP/1.1 正向与 CONNECT、TLS 保护的 HTTP/CONNECT、SOCKS5 无认证 TCP。流式背压、有界缓冲、全局/入口并发与字节速率、握手/连接/空闲超时、启停和关闭回收。无可用上游失败，不隐式直连；不重放业务请求。
3. 将实际入口、资源限制、启停与接入说明贯穿 API、独立 CLI 和共享 React/Tailwind MPA。配置持久化编辑与版本发布仍归 T05；本阶段使用启动 TOML，运行启停明确不写回文件。
4. 通过本地真实 socket/TLS 服务器验证入出站协议矩阵、HTTP framing/大请求与响应、上游认证隔离、到期拒绝、半关闭、取消、IPv6、超时、连接/速率限制和关闭回收。补充 Daemon/API/CLI/浏览器集成证据。

每片经过相关测试后提交。Issue 未满足全部验收前保持 open；没有真实 eBPF/供应商证据时不宣称正式版本完成。

## 协议边界

- HTTPS 目标通过 CONNECT 保持端到端 TLS；HTTPS 代理指客户端与代理间的 TLS，两者分别验收。
- HTTP 转发使用 Hyper 的 HTTP/1.1 framing；删除逐跳头和客户端代理凭据，正确生成目标 Host，不缓冲整份请求/响应。
- 上游 HTTP/HTTPS 的普通 HTTP 请求使用 absolute-form；隧道使用 CONNECT。SOCKS5 保留域名由上游解析，支持 IPv4/IPv6。上游不成功时返回本项目网关错误，不把 407 转成客户端登录挑战。
- SOCKS5 入站只提供无认证 CONNECT，BIND/UDP ASSOCIATE 显式拒绝；UDP 属于 T19，透明 UDP 仍属于 T15。
- 第一版只协商 HTTP/1.1，不宣传 HTTP/2/3 或未经验证的 HTTP Upgrade。
- 入站不增加账号、认证或来源 ACL；上游凭据、服务器 TLS 证书和操作系统权限仍按协议使用。

参考：[HTTP/1.1 RFC 9112](https://www.rfc-editor.org/rfc/rfc9112.html)、[SOCKS5 RFC 1928](https://www.rfc-editor.org/rfc/rfc1928.html)、[Tokio rustls](https://docs.rs/tokio-rustls/)。最终以锁定依赖与实际测试结果为准。
