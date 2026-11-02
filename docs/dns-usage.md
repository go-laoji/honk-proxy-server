# DNS 使用与运行边界

DNS 由独立 Rust 模块实现，API、CLI、Desktop 与显式代理共用同一运行版本。支持 system、UDP、TCP、DoT、DoH 上游及按域名、查询类型、解析用途选择上游。默认使用操作系统解析，不开启 DNS 监听或 Linux 捕获。

## 配置与引导

主配置使用 `[dns]`，例如下面的 UDP 配置。地址必须是明确的 IP 和端口，避免先解析 DNS 上游自身造成循环。

```toml
[dns]
enabled = true
default_upstream = "primary"
timeout_ms = 5000
max_parallel = 64
cache_entries = 2048
cache_bytes = 16777216
# 显式需要本地 DNS 服务时再设置，同时监听 UDP/TCP。
# listen = "127.0.0.1:1053"

[[dns.upstreams]]
id = "primary"
protocol = "udp"
addresses = ["1.1.1.1:53", "[2606:4700:4700::1111]:53"]

[[dns.routes]]
id = "example"
domain = "example.com"
suffix = true
query_types = ["A", "AAAA"]
purposes = ["business"]
upstream = "primary"
```

路由从上到下首条命中，后缀按域名标签边界匹配；空类型/用途列表表示不限。用途包括 `bootstrap`、`provider`、`health`、`business`。域名使用 ASCII/Punycode，允许 SRV/TXT 名称中的下划线。System 只支持 A/AAAA，没有 DNS wire TTL，结果不进入缓存；配置 DNS 监听或捕获时不能保留 system 上游。

DoT 使用 `protocol="tls"`，DoH 使用 `protocol="https"`；两者必须设置用于证书验证的 `server_name`，地址仍为明确的 IP:port。DoH 默认路径 `/dns-query`，使用验证 TLS 的 HTTP/1.1 POST，不跟随重定向、不读取环境代理。需要私有 CA 时设置 `ca_certificate`，不支持跳过证书验证。

TCP、DoT、DoH 可以用 `proxy_id` 指定已启用的静态 HTTP/HTTPS/SOCKS5 节点；域名形式的代理端点还必须设置 `proxy_bootstrap_ips`，同一代理的列表必须一致。UDP 不借用 TCP-only 代理能力。DNS 出口不从短效池临时购买节点，出口失败不会隐式改成直连。配置准备阶段会拒绝指向自身 DNS 监听或代理入口的循环。

共享解析用于代理端点、供应商接口、健康目标与本地/捕获 DNS。显式代理的业务目标域名仍交给所选上游代理解析；这有助于保留 HTTP CONNECT/SOCKS5 的远端解析语义，不能把本地 DNS 的记录误称为全部业务站点的解析记录。

## CLI、API 与共享页面

```sh
hps dns show
hps dns show --limit 50
hps --timeout 65 dns resolve example.com --query-type A --purpose business
hps dns context 203.0.113.9
hps dns clear --domain example.com
hps dns clear
hps dns save dns-policy.toml --expected-revision 3
```

单独保存的 `dns-policy.toml` 从 DNS 策略根开始，不包含 `[dns]` 外层。保存使用期望版本检查；过期编辑返回冲突，失败保留运行版本。CLI 默认请求超时 10 秒，可通过全局 `--timeout` 调整；Desktop DNS 查询允许 65 秒，覆盖配置的最长 60 秒解析超时。

| 方法 | 路径 | 功能 |
| --- | --- | --- |
| GET | `/api/v1/dns?after=...&limit=50` | 配置、运行版本、资源统计与缓存分页，最多 200 条 |
| POST | `/api/v1/dns/resolve` | `{name,query_type,purpose}`，返回实际选路、传输、结果及 TTL |
| GET | `/api/v1/dns/context?ip=203.0.113.9` | 有时效和来源的域名候选或未知 |
| POST | `/api/v1/dns/cache/clear` | `{domain:null}` 清全部，或指定精确域名 |
| POST | `/api/v1/dns/save` | `{configuration,expected_revision}`，准备并激活新配置 |

`/console/dns.html` 是独立 HTML 页面，提供策略编辑、实际解析诊断、缓存分页/清理、域名上下文和 Linux 捕获统计。浏览器同源 API 与 Desktop 原生桥访问同一契约；无项目登录、账号或权限验证。

## 缓存、上下文与资源

正缓存使用回答中的 TTL；NXDOMAIN/NODATA 负缓存遵守 SOA TTL 与 MINIMUM 的较小值，并限制 CNAME 链。DoH 的 HTTP Age 会扣减 DNS TTL，发给客户端的 TTL 随时间减少。零 TTL、超时、SERVFAIL 和 system 结果不缓存。缓存按实际路由、用途、出口及信任内容隔离，热更新不会复用旧策略结果。使用单调时钟，清理缓存后旧在途响应不能重新填回被清理的代际。

同一查询的在途请求合并，客户端离开后已受理工作仍有服务端超时和关闭控制。并发上限跨排空中的旧版本共同生效，降低限额不会额外放行旧工作。System 的不可取消 libc 解析固定使用 8 个工作线程和最多 32 个排队请求；超时任务不会无限扩张线程或队列。统计明确显示占用。

默认缓存 2048 条/16 MiB；最大 100000 条/256 MiB，并发 1–256。UDP 查询/EDNS 有界，截断响应转 TCP；TCP 客户端和服务器任务数量、连接寿命、帧长及 DoH 头/正文均有限制。Daemon 关闭取消并等待自身异步 DNS 工作，旧版本和监听任务有排空预算。

域名上下文只接受未过期缓存中与问题或 CNAME 链关联的 IP，忽略无关 Answer 记录；共享 IP 返回多个候选，不强行选择一个。没有观测时返回未知。外部 DoH/DoT、ECH 及网关客户端进程不能据此解密或推断；不生成目标站点 FakeIP。

## Linux eBPF DNS 捕获

需要本项目自有对象、可操作的 cgroup v2 和相应 Linux 权限。实际 DNS hook 已在 aarch64 Linux 6.12.115 LTS 与 7.0.14 验证，使用的 cgroup socket-address helper 要求 Linux 6.10 或以上；T03 的基础预检通过不代表 DNS hook 可用。

```toml
[dns.capture]
object = "ebpf/target/bpfel-unknown-none/release/hps-ebpf"
cgroup = "/sys/fs/cgroup/hps-clients"
port = 1053
```

与上面的 wire DNS 配置组合使用，`port` 为专用捕获端口，不能为 0、53 或与入口、DNS 显式监听及上游端口冲突。配置对象相对路径按配置文件目录解析。构建自有对象和前置环境说明见 [eBPF 使用文档](linux-ebpf-usage.md)。

自研 connect/sendmsg/recvmsg/getpeername 的 IPv4/IPv6 共八个 hook 接管选定同 netns cgroup 中 TCP/UDP 53，排除 Daemon 自身。IPv4 `127.77.0.0/16` 与 IPv6 `fd77:6870:7300::/112` 仅用于保留原 DNS 对端身份，回复源地址恢复为客户端原本查询的 DNS 地址；单 UDP socket 同时查询多个服务器也不会串线。原 DNS 上游策略由 Daemon 的 DNS 配置决定。

对象校验、真实 verifier、双栈监听和禁用态 attachment 都在配置提交前完成，提交后才启用重定向。相同 capture 配置下，DNS 路由/上游/缓存参数可热更新并复用 hook。修改或关闭已有 capture 返回 `pending_restart`，保留旧别名表直到重启；重启后旧 connected DNS socket 需要重新建立。

每个 netns 的私有 OS 锁、program/map FD 与精确标记的 IPv6 路由由本实例持有，失败及关闭回收自己的对象，不移除其他网络对象。peer 映射、别名分配与服务器任务均有界；容量耗尽或内核映射失败显式报错，不能悄悄漏到原 DNS。捕获状态和真实 hook/map/计数可在 DNS 页面查看。

这完成 DNS 接管，完整 TCP/UDP 透明转发由 T15 独立交付，Linux 页的完整数据面 ready 在其完成前仍为 false。实际内核证据见 [DNS 捕获验收](validation/linux-dns-capture-2026-11-01.md)。
