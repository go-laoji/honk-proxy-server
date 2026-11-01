# 显式代理接入与运行语义

本项目自主实现 Rust HTTP/HTTPS/SOCKS5 核心。honk 只作能力学习参考，不作为依赖。此文对应 T04；供应商 API 提取、健康探测、规则分组及 Linux eBPF 分别由后续任务交付。

## 配置与启动

复制 `configs/hps.proxy.example.toml`，将 upstream 替换为实际供应商端点并启用。静态上游是当前核心验证路径，短效节点须填写供应商实际 `expires_at_unix`；不可将续期后的同一 IP 自动当作原租约延长。

```sh
./target/debug/hps config check configs/hps.proxy.example.toml
./target/debug/hpsd --config configs/hps.proxy.example.toml
./target/debug/hps --output json inbound list
./target/debug/hps inbound stop browser-http
./target/debug/hps inbound start browser-http
```

默认示例只监听本机。需远程接入时在 TOML 明确填写可达的监听地址。管理 API、Web、代理入口均不要求本项目账号、密码或来源权限；上游认证由服务端完成。

配置、证书和资源限制从启动 TOML 读取，修改后重启 Daemon 生效。TLS 文件的相对路径基于 TOML 所在目录。入口启停立即生效，停止会关闭该入口的现有连接；运行启停不写回 TOML，重启后按 `enabled` 恢复。配置持久化编辑、版本与回滚属于 T05。

## 浏览器、系统与 curl

HTTP 入口可同时访问 HTTP 和 HTTPS 目标，后者经 CONNECT 保留客户端到站点的端到端 TLS。本项目不解密 HTTPS 正文。

```sh
curl --proxy http://127.0.0.1:17901 http://example.com/
curl --proxy http://127.0.0.1:17901 https://example.com/
curl --proxy socks5h://127.0.0.1:17903 https://example.com/
```

浏览器/系统中填写 HTTP 代理地址与端口，并将 HTTPS 目标设为使用同一代理。Chromium 的策略或 `--proxy-server=http://127.0.0.1:17901` 可选择该入口。系统界面的“HTTPS 代理”有时指 HTTPS 目标使用的 HTTP 代理，不自动代表与代理之间使用 TLS。[Chromium 代理文档](https://chromium.googlesource.com/chromium/src/+/HEAD/net/docs/proxy.md)

HTTPS 代理入口是客户端先与本项目建立 TLS，再发 HTTP/CONNECT。启用示例中的 `browser-https`，提供匹配访问名称的证书链和私钥；客户端须信任签发 CA。curl 的对应形式为：

```sh
curl --proxy https://proxy.example:17902 \
  --proxy-cacert /client/path/proxy-ca.pem https://example.com/
```

上游 HTTPS 代理默认使用公开根证书库验证，可配置额外 CA 和证书名称，不提供跳过验证选项。HTTP/1.1 是本阶段验证的 TLS 代理协议，不宣传 H2/H3。证书是否处于有效期、起止时间通过入口 API/页面展示，校验证书域名、链与信任仍由 TLS 客户端完成。

SOCKS5 仅提供无认证 CONNECT，支持 IPv4、IPv6 和域名。BIND/UDP ASSOCIATE 返回不支持；只提供用户名密码认证方法的客户端收到无可接受方法。`socks5h` 将目标域名交给本项目再传至上游；HTTP/CONNECT 的域名也传至所选上游。本项目当前仅通过系统解析器解析上游端点本身；完整 DNS 策略在 T09。浏览器自身的 DoH、DNS、PAC 和绕过策略可能影响哪些流量进入代理，需按客户端配置判断。

## 选择、失败与资源限制

- 静态节点按轮转游标选择；禁用、过期、剩余寿命不大于准入窗口的节点被排除。此筛选不代表健康探测通过，完整组策略由 T08/T10 交付。
- 没有合格上游时 HTTP/CONNECT 返回 503，SOCKS5 返回网络不可达。不会隐式直连目标。
- 上游握手超时返回 HTTP 504，握手/认证或响应失败返回 502。上游 407 不转成客户端账号挑战；HTTP 转发删除客户端代理凭据、逐跳头并按实际目标生成 Host。CONNECT/SOCKS5 隧道保持不透明，不能也不会修改端到端加密内容里的头部。
- 每次请求/隧道只选一次节点，不因失败换节点重放业务数据；收费 POST 在上游提交后丢失响应也只返回错误。重定向由客户端处理，本服务不自动跟随。
- 全局和每入口连接上限覆盖握手及活跃隧道；满额立即关闭新连接并累计对应限制命中计数，不创建无界等待队列。
- 字节速率是全局/每入口共享的双向业务吞吐上限，0 表示不限。HTTP 请求/响应体和隧道数据使用有界分块与背压，TLS/HTTP 握手开销不计入业务限速。主动限速等待不算空闲；缓冲大小不是整份消息大小限制。
- 握手超时限制 TLS/SOCKS/HTTP 请求头，连接超时包括到上游的 DNS/TCP/TLS/代理握手，空闲超时回收无进展连接。TCP 双向半关闭保留反向响应；停止入口或关闭 Daemon 会取消转发和驱动任务、释放连接额度。
- HTTP Upgrade 显式返回 501；SOCKS5 UDP 和 Linux 透明 UDP 各有独立验收，不由 TCP 测试代替。

`/healthz` 只代表管理进程存活。`/readyz` 为 200 表示至少一个入口运行、HTTPS 入口证书处于有效期且有可准入的配置上游，不代表上游已通过健康探测；配置为空、入口停止、租约到期或正在关闭时返回 503。Linux eBPF 的 compiled/attached/ready 单独报告，显式代理可用不能证明已接管 Linux 流量。
