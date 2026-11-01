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

## 捕获代际与服务级验收（2026-11-02）

已接入 `dns.capture = { object, cgroup, port }`，默认仍停用。对象/信任、内核 verifier、双栈监听与禁用状态的 hook attachment 均在修改配置 head 前准备；提交后才开启 redirect map。相同捕获配置的 DNS 上游、路由、缓存参数热更新复用 hook，错误对象或端口冲突保持原 head。

修改已有捕获的 cgroup、端口、对象内容或关闭捕获统一返回 `pending_restart`，不会在仍有地址被改写的 DNS socket 时强行更换别名表。Daemon 重启会释放旧捕获，使用旧 connected DNS socket 的应用需要重新建立 socket。此限制与普通显式代理设置独立；完整透明数据面仍由 T15 实现。

服务级测试在真实 7.0 和 QEMU 6.12 再次运行，覆盖 Supervisor boot、真实双栈 TCP/UDP、热策略保留 program IDs、关闭捕获待重启、取消待重启配置、错误对象和端口冲突保持 head、shutdown 后重新启动取得所有权。原始证据见 `docs/validation/linux-dns-application-kernel*-2026-11-02.json`。最终跨平台回归和使用文档继续进行，Issue #14 保持 open。

健康目标的 bootstrap 预检改为独立的 5 秒调度节奏，系统 DNS 回答仍不缓存、不虚构 TTL；策略变更与 bootstrap 状态发布协调，重新确认目标前不自动购买新节点。停用补货仍优先报告 disabled，避免被健康预检状态覆盖。
