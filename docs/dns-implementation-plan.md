# T09 DNS 实施计划

对应 [Issue #14](https://github.com/go-laoji/honk-proxy-server/issues/14)，基于已验收的 T03/T04/T05/T08。本次继续独立 Rust 实现，不集成 honk，不引入项目账号或权限层。

## 提交切片

1. DNS 配置/DTO 与轻量网络接口：共享 BoxIo、解析用途和拨号契约，避免 DNS 与代理引擎循环依赖。支持 system、UDP、TCP、DoT、DoH 上游；按域名/类型/用途选路，默认 system 兼容既有显式代理。监听和捕获默认停用，跨平台只在显式配置时开启。
2. Rust 解析核心：使用 Hickory 的线格式模型，自己管理有界传输、并发、缓存和策略。验证 ID/问题/来源，UDP TC 转 TCP，DoT/DoH 验证证书与 bootstrap；静态 HTTP/HTTPS/SOCKS5 节点可承载 TCP DNS，UDP 不伪装成 TCP-only 节点能力。代理地址和 DNS 地址需要明确引导，监听/捕获模式拒绝递归 system 配置。
3. 正负 TTL、CNAME/问题类型、出口/路由/策略指纹隔离、单调过期、在途合并、清理代际、字节/条数/并发上限。system 不提供 DNS TTL，结果明确不缓存，不虚构 TTL。缓存清理不允许在途旧结果重新填回已清理的代际。
4. 应用代际协调、UDP/TCP DNS 服务、共享代理端点/健康解析；API/CLI/React Tailwind 真正 MPA 的上游/路由编辑、诊断、缓存分页和清理。保存配置仍走版本冲突与预备后激活，DNS 故障不触发隐式业务直连。
5. 实际 Linux DNS 接管：扩展自有 cgroup connect/sendmsg/recvmsg/getpeername hook，以有界 resolver 地址别名映射保留 IPv4/IPv6、TCP/UDP 的远端身份，避免单个 UDP socket 并行查询多个 DNS 服务器时回复源混淆。IPv4 使用专用 loopback 前缀，IPv6 使用自有本地前缀路由；回复保留接收目标地址。机器级所有权、同 netns 约束、排除本 Daemon、准备/回收均验证。DNS 返回真实上游结果，这些别名不是目标站点 FakeIP。

## 验收

本地真实 UDP/TCP/DoT/DoH 与代理穿透；IPv4/IPv6、正负 TTL/零 TTL、错误响应/超时/截断、缓存隔离与清理、配置冲突/热更新和防递归；HTTP/HTTPS/SOCKS5 旧回归继续通过。Linux 在实际 7.0 和 6.12 LTS 验证 DNS 包接管、单 socket 多上游并行及源地址恢复，未完成时保持 Issue open。

只有显式域名或有时效、来源可解释的 DNS 观测才作为域名上下文；多个候选、未观察到 DNS、外部 DoH/DoT、ECH 等情况回退为 IP/未知，不伪造网关客户端进程或域名。生产全流量透明转发仍由 T15 交付。

参考 [Hickory 0.26](https://docs.rs/hickory-proto/latest/hickory_proto/)、[负缓存 RFC 2308](https://www.rfc-editor.org/rfc/rfc2308.html)、[DoH RFC 8484](https://www.rfc-editor.org/rfc/rfc8484.html)。
