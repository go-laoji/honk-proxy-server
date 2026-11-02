# Linux 透明 TCP/UDP 使用说明

本项目自行实现 Rust/Aya cgroup + TCX + transparent socket 数据面，不集成 honk。浏览器仍可使用 HTTP/HTTPS CONNECT/SOCKS5 显式入口；Linux 透明捕获可按明确选择的本机 cgroup 或 LAN 网卡接入同一个规则、分组、租约、健康、连接和带宽引擎。默认关闭。

## 支持与构建

已实际验证的小端架构是 aarch64、x86_64，内核为 6.12.115 LTS、7.2.14；另有 Linux 7.0.14/aarch64 回归。最低**已验证**内核版本为 6.12.115，不将更旧版本、其他架构或 honk 的支持范围视作本项目保证。程序实际 verifier/load/attach、接口、路由、监听和代际检查失败时，不报告捕获就绪。

常规服务不需要桌面或 nightly：

```sh
cargo build --locked -p hps-cli -p hps-daemon --no-default-features
```

eBPF 对象在独立 `ebpf/` 工作区构建，需其固定 nightly、rust-src 和 bpf-linker：

```sh
rustup toolchain install nightly-2026-09-01 --component rust-src --profile minimal
./scripts/build-ebpf.sh
```

构建步骤及 bpf-linker 安装见 [eBPF 使用说明](linux-ebpf-usage.md)。本轮业务对象使用冻结的 ABI v4 标记；旧 ABI v3 对象在挂载前明确拒绝。不要把 CLI、Daemon 或 ELF 对象互相代替。

## 配置明确范围

在已有上游/规则配置中加入以下表。不要将示例路径当成自动创建的目录；由部署者创建并选择目标 cgroup v2，把需要接管的应用放入该范围。Daemon 需要实际加载自有 BPF、策略路由及透明 socket 的系统能力；WebView 只连接管理 API。

```toml
[linux_capture]
object = "/opt/hps/hps-ebpf"
local_cgroup = "/sys/fs/cgroup/hps-clients"
lan_interfaces = []
ipv4 = true
ipv6 = true
tcp = true
udp = true
tcp_port = 18080
udp_port = 18080
max_flows = 8192
max_pending = 128
max_udp_sessions = 1024
udp_idle_secs = 60
max_datagram_bytes = 65507
max_queued_datagrams = 256
max_queued_bytes = 4194304
```

网关部署可省略 `local_cgroup` 并显式设置 `lan_interfaces = ["lan0"]`；也可同时启用两者。服务不猜测所有网卡或接管整机。LAN 进程始终未知，只有实际捕获的本机 socket 提供 PID、comm、cgroup 和 cookie 身份。显式代理客户端也不能声明生产进程身份；规则模拟中的声明仅是假设。

缺省旁路包含 loopback、链路本地、组播等特殊地址。可用 `bypass_cidrs`、`bypass_ports` 覆盖为需要的明确集合。Daemon 出站和透明回复旁路自身；启用 DNS 捕获时自动补齐自己的双栈 DNS alias 前缀。默认路由身份为 mark `0x48500000`、bypass mark `0x48500001`、表 `10530`、优先级 `31530`，可通过配置调整，冲突时拒绝接管，不 replace 外部对象。

规则动作沿用节点、分组、明确 Direct/Reject。HTTP/HTTPS 上游只承载 TCP，UDP 使用明确 Direct 或实际 SOCKS5 UDP association；不能以 TCP CONNECT 冒充 UDP。节点仍受租约、分组、预算和各传输健康证据约束；当前独立主动健康探测的 UDP 项如实标记 unsupported，UDP SOCKS5 真实数据面验证使用明确关闭健康检查的配置，不虚构 UDP 探测成功。

## 启停、候选与实际状态

CLI 查询目标服务，不在 CLI 所在机器隐式执行远端网络操作：

```sh
hps --endpoint http://127.0.0.1:17900 --output json linux capture status
hps --endpoint http://127.0.0.1:17900 --output json linux capture control stop --expected-revision 1 --expected-epoch <当前epoch>
hps --endpoint http://127.0.0.1:17900 --output json linux capture control start --expected-revision 1 --expected-epoch <当前epoch>
hps --endpoint http://127.0.0.1:17900 --output json linux capture control recheck --expected-revision 1 --expected-epoch <当前epoch>
hps --endpoint http://127.0.0.1:17900 --output json linux capture save capture.json --expected-revision 1 --expected-epoch <当前epoch>
```

`capture.json` 是完整 Linux 捕获配置对象；JSON `null` 表示持久禁用候选。保存只创建配置版本，后续在配置工作区审阅并激活。对象、范围、路由身份、端口或预算变化需要重启，不能把“已保存期望版本”显示为正在运行。普通规则、分组和节点更新可热发布。

`linux.html` 是浏览器与 Desktop 共用的实际 MPA：显示运行/期望版本、用户态/内核代际、监听、链接、网卡启用/链路状态、路由、范围、map/program ID、资源与拒绝计数。可启停、重新检查并恢复、保存候选、跳转配置及任务。非 Linux 服务明确禁用捕获操作；Mac Desktop 可管理远端 Linux 服务。

API 为 `GET /api/v1/linux/capture`、`POST /api/v1/linux/capture/control`、`POST /api/v1/linux/capture/save`。控制请求携带 `expected_epoch` 与 `expected_revision` 防止旧界面操作错误实例；这是一致性检查，项目没有登录、账号或权限层。手动操作进入已有持久任务/事件，客户端离开不取消已受理工作。

捕获对象就绪与有可用转发路径分别核验。`/readyz` 同时区分 `explicit_ready` 和 `transparent_ready`；无合格出口时不隐式直连。停止透明捕获只结束自己的透明流，显式入口继续运行。临时停止不改变持久配置，重启按期望配置运行；持久禁用需要保存、激活禁用候选并重启。

## 故障、代际与边界

- 新流根据内核记录的 immutable revision 使用对应引擎；已建立 TCP/UDP 保留旧代际。旧 pending 元数据与业务 owner 都退休后才释放快照。代际、流、待接入、UDP session 和排队字节均有上限。
- 实际 cgroup/TCX、接口身份与启用/链路状态、路由或策略丢失时降级，先暂停所选范围的新流，再最多三次退避原位修复。修复保持暂停，结束旧 owner、执行网络 RCU 屏障、排空旧 backlog、精确退休元数据，接收循环和引擎就绪后再恢复。外部归属冲突、替换的 cgroup 或网卡身份不能被静默接管。
- 自动恢复到限后保持降级。恢复原接口/对象后可手动重新检查；重新选择范围或新对象走配置激活和重启。正常停止和异常启动清理只针对本实例精确对象，保留外部规则。
- SIGKILL 会使 FD link 与 socket 被内核释放；下一 Daemon 根据命名空间锁和私有日志回收自有 route/rule 残留，恢复 SQLite 配置。服务管理器负责拉起进程。本轮没有提供独立常驻的严格崩溃 kill switch；该项列在 LINUX-07 的独立验收范围。
- UDP 不重放业务包；同 socket 不能混用捕获与旁路类别。IPv4 分片、非 atomic IPv6 分片等未支持包在所选范围明确拒绝并计数。最大数据报预算不保证外部链路 MTU；TCP 大块传输已实际验证，不声称支持所有 offload/扩展报文形态。

实际测试范围、逐批限制和可复现证据见 [T15 验收](validation/linux-transparent-2026-11-02.md)。付费供应商账号调用仍未验证，快代理使用文档契约/故障测试，免费来源与健康判断分别记录；Linux 内核验收均为真实程序和数据包。
