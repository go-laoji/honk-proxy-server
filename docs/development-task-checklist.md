# 开发任务与交付清单

- 日期：2026-11-01。
- 总任务：[Epic #5](https://github.com/go-laoji/honk-proxy-server/issues/5)。
- 基准：[技术规划](technical-architecture-plan.md)、[99 项控制台功能](console-feature-checklist.md)。
- 机器可读映射：[development-tasks.json](development-tasks.json)。

## 1. 范围与执行规则

全部后端使用 Rust；交付 hps CLI、hpsd Daemon 和 Tauri Desktop；React + Tailwind 的 18 个真实 HTML 页面共用同一产物。核心 HTTP/HTTPS/SOCKS5、短效提取 API、租约和补池预算不改变。当前项目无登录、账号与权限验证；所有可达使用者可使用全部功能。

21 个任务覆盖 99 项功能，每个功能只有一个主要交付任务：80 个 P0、15 个 P1、4 个 P2。工程基础任务 T02 和发行任务 T17 另列通用验收，不代替各业务功能的完成。

功能分支使用 git flow 开启，小步提交；完成一个 Issue 先回复结果及验证再关闭。不自动推送、合并或结束功能分支。文档和任务状态只标记已实际验证的进展。

## 2. 依赖与开发顺序

先完成 T01 独立服务基础及 T02 共享页面骨架；同步尽早准备 T03 的真实 Linux 验证环境。T04 自研显式代理可独立推进，与 T03 共享流量接口；不会因只完成 CLI 或 Web 骨架就声称 M0/正式 Linux 版本完成。

T05 配置基础后推进 T06/T07 短效 API 与补池。T08–T14 逐步闭合健康、DNS、选点、规则、节点、连接及任务链；T15/T16 完成 Linux 与桌面生命周期，T17 统一验收。T18–T21 为增强或储备。

当前主机为 macOS，已有 Docker Linux 可供环境预检；容器内核/BTF/权限与真实数据面需实际检查，不能预先视为满足条件。用户已提供快代理官方文档与云代理免费源，并说明没有采购，可按文档实现。供应商验收采用文档契约、故障服务与免费源实测；付费账号调用明确标为未验证，不要求采购。Linux 真实 eBPF 门槛保持不变。

| 任务 | 优先级 | Issue 与工作项 | 依赖 | 状态 |
| --- | --- | --- | --- | --- |
| T01 | P0 | [#6](https://github.com/go-laoji/honk-proxy-server/issues/6) Rust 工作区与独立 CLI/Daemon 基础 | 无 | 已完成基础交付 |
| T02 | P0 | [#7](https://github.com/go-laoji/honk-proxy-server/issues/7) 共享 React Tailwind MPA 与 Desktop 桥骨架 | T01 | 已完成基础验收 |
| T03 | P0 | [#8](https://github.com/go-laoji/honk-proxy-server/issues/8) 自研 Rust 核心与 Linux eBPF 前置验证 | T01 | 已完成，实际 6.12/7.0 内核已验收 |
| T04 | P0 | [#9](https://github.com/go-laoji/honk-proxy-server/issues/9) HTTP HTTPS SOCKS5 核心代理与资源控制 | T01 | 已完成显式代理验收 |
| T05 | P0 | [#10](https://github.com/go-laoji/honk-proxy-server/issues/10) SQLite 配置版本、校验、激活与回滚 | T01 | 已完成持久化与三端验收 |
| T06 | P0 | [#11](https://github.com/go-laoji/honk-proxy-server/issues/11) 短效提取 API、批次租约与安全重试 | T05 | 已完成契约、租约和三端验收 |
| T07 | P0 | [#12](https://github.com/go-laoji/honk-proxy-server/issues/12) 到期预测、水位与预算驱动补池 | T06 | 已完成预算、补池与运行验收 |
| T08 | P0 | [#13](https://github.com/go-laoji/honk-proxy-server/issues/13) 统一健康检测与真实流量反馈 | T04, T06 | 已完成 |
| T09 | P0 | [#14](https://github.com/go-laoji/honk-proxy-server/issues/14) DNS 上游、路由、缓存与诊断 | T03, T05, T04 | 已完成真实内核、CLI/MPA 与 Desktop 窗口验收 |
| T10 | P0 | [#15](https://github.com/go-laoji/honk-proxy-server/issues/15) 节点分组、选点、固定与租约准入 | T04, T06, T08 | 已完成三端与真实固定出口验收 |
| T11 | P0 | [#16](https://github.com/go-laoji/honk-proxy-server/issues/16) 规则 CRUD、模拟与一致性发布 | T05, T09, T10 | 已完成，实际进程/模拟/内核代际联合通过 |
| T12 | P0 | [#17](https://github.com/go-laoji/honk-proxy-server/issues/17) 节点管理与来源详情控制台闭环 | T02, T06, T08, T10 | 已完成三端、排空与真实连接关联验收 |
| T13 | P0 | [#18](https://github.com/go-laoji/honk-proxy-server/issues/18) 活跃连接、流量统计与路由解释 | T04, T10, T11 | 已完成，透明流/来源/出口/字节与代际联合通过 |
| T14 | P0 | [#19](https://github.com/go-laoji/honk-proxy-server/issues/19) 后台任务、SSE 与结构化事件 | T05, T06 | 已完成持久任务、三端事件流与实际窗口验收 |
| T15 | P0 | [#20](https://github.com/go-laoji/honk-proxy-server/issues/20) Linux 透明数据面产品化与恢复 | T03, T09, T11, T13, T14 | 已完成，双架构/双内核 52 项实际用例与 CLI/MPA 通过 |
| T16 | P0 | [#21](https://github.com/go-laoji/honk-proxy-server/issues/21) Desktop 本地远程档案与系统服务集成 | T02, T05, T15 | 待开发 |
| T17 | P0 | [#22](https://github.com/go-laoji/honk-proxy-server/issues/22) 分包发行、端到端与发布门槛 | T01, T02, T04, T05, T06, T07, T08, T09, T10, T11, T12, T13, T14, T15, T16 | 待开发 |
| T18 | P1 | [#23](https://github.com/go-laoji/honk-proxy-server/issues/23) 历史观测、诊断导出与任务增强 | T17 | 待开发 |
| T19 | P1 | [#24](https://github.com/go-laoji/honk-proxy-server/issues/24) SOCKS5 UDP、订阅与高级分组扩展 | T17 | 待开发 |
| T20 | P1 | [#25](https://github.com/go-laoji/honk-proxy-server/issues/25) 备份恢复、系统体验与高级 Linux 诊断 | T17 | 待开发 |
| T21 | P2 | [#26](https://github.com/go-laoji/honk-proxy-server/issues/26) 多实例与其他平台等储备范围 | T17 | 待开发 |

## 3. 任务验收详情

### T01 Rust 工作区与独立 CLI/Daemon 基础

- Issue：[#6](https://github.com/go-laoji/honk-proxy-server/issues/6)。
- 功能：INST-01、INST-02、INST-04、DASH-01、SYS-04。
- 工作：建立独立可构建的 Rust CLI/Daemon、统一 DTO/API client、启动配置与结构化错误；无账号直接提供 status/version/capabilities；明确管理就绪与代理就绪。
- 验收：cargo test/clippy 通过；不编译 Tauri 可运行 hpsd 与 hps；真实 HTTP 请求与 CLI JSON 输出验证；无数据面时 proxy_ready=false。

### T02 共享 React Tailwind MPA 与 Desktop 桥骨架

- Issue：[#7](https://github.com/go-laoji/honk-proxy-server/issues/7)。
- 功能：工程基础或发行验收，无重复功能归属。
- 工作：接管现有前端模板，建立18个真实HTML入口、共用布局/主题/API transport和页面清单；Rust Daemon与Tauri嵌入同一dist；Desktop原生桥处理目标连接，浏览器同源API，无登录。
- 验收：前端类型/生产构建通过；18页独立可访问、刷新、无登录、未知页404；两端同一web_build_id；Desktop cargo check通过，基础页面连接真实Daemon。

### T03 自研 Rust 核心与 Linux eBPF 前置验证

- Issue：[#8](https://github.com/go-laoji/honk-proxy-server/issues/8)。
- 功能：LINUX-01、LINUX-02。
- 工作：定义本项目流量/拨号接口；自研Rust/Aya eBPF程序、公共ABI和loader；实现只读环境预检，验证真实挂载/maps与回收。不集成honk，初始Linux验证基线不等同最终最低内核要求。
- 验收：真实Linux上的自有eBPF程序挂载、map交互与清理证据；预检不等于接管；透明TCP/UDP出口由T04/T15联合验收。非Linux如实报告；无honk依赖、对象或fork。

### T04 HTTP HTTPS SOCKS5 核心代理与资源控制

- Issue：[#9](https://github.com/go-laoji/honk-proxy-server/issues/9)。
- 功能：IN-01、IN-03、IN-04、IN-05、IN-06、SYS-02。
- 工作：无认证HTTP正向/CONNECT、TLS代理与SOCKS5 TCP；HTTP/HTTPS/SOCKS5上游；统一拨号与连接准入，流式转发、半关闭、超时/资源限制和接入说明。
- 验收：协议矩阵真实互通；无可用上游不隐式直连；上游凭据不泄漏；大文件、取消、IPv6与业务数据不重放回归通过。

### T05 SQLite 配置版本、校验、激活与回滚

- Issue：[#10](https://github.com/go-laoji/honk-proxy-server/issues/10)。
- 功能：CFG-01、CFG-02、CFG-03、CFG-04、CFG-05、CFG-06、SYS-01、SYS-03。
- 工作：单写者SQLite、迁移、草稿/配置版本/运行激活状态；API/CLI及共享页面；监听/TLS/日志/存储配置，不含账号表。
- 验收：持久化与重启恢复、过期版本409、失败保留旧版本、脱敏导入导出、回滚一致性；HTTP无账号可读写。

### T06 短效提取 API、批次租约与安全重试

- Issue：[#11](https://github.com/go-laoji/honk-proxy-server/issues/11)。
- 功能：PROV-01、PROV-02、PROV-04、PROV-05、PROV-06、PROV-07、PROV-08、PROV-10、PROV-11、PROV-12。
- 工作：适配真实供应商与可复现测试服务；配置提取参数/签名、TTL起算、增量批次/快照、凭据/白名单/配额；持久记录结果不确定，提供API/CLI/页面。
- 验收：新批次不误删有效旧节点；到期不延长；损坏/空结果不清池；响应丢失不盲目重复消耗额度；按用户提供的快代理契约验收，免费源实测；付费调用未验证且不要求采购。

### T07 到期预测、水位与预算驱动补池

- Issue：[#12](https://github.com/go-laoji/honk-proxy-server/issues/12)。
- 功能：PROV-03、PROV-09。
- 工作：单来源控制器管理最小/目标/最大水位、批量、预退休窗口、最小剩余寿命、在途预算、限频/退避和按协议/地域补充。
- 验收：并发触发合并；在途计预算；空闲不无限购买；短缺不每请求直调供应商；临界到期/重启/429/无配额测试。

### T08 统一健康检测与真实流量反馈

- Issue：[#13](https://github.com/go-laoji/honk-proxy-server/issues/13)。
- 功能：HEALTH-01、HEALTH-02、HEALTH-03、HEALTH-04、HEALTH-05、NODE-05。
- 工作：按TCP/UDP、IPv4/IPv6区分健康；寿命约束的准入探测、预算/退避、单个批量任务和实际连接反馈；API/CLI/页面。
- 验收：端口/认证/穿透分层；目标403/429不全局判死；短寿命不浪费探测；所有入口单一健康状态，限并发与任务恢复。

### T09 DNS 上游、路由、缓存与诊断

- Issue：[#14](https://github.com/go-laoji/honk-proxy-server/issues/14)。
- 功能：DNS-01、DNS-02、DNS-03、DNS-04。
- 工作：共享DNS解析与bootstrap、出口策略和缓存隔离；域名上下文、诊断与清理API/CLI/页面；防下载/代理解析回环。
- 验收：IPv4/IPv6、TTL/缓存、不可见域名回退明确；真实Linux DNS接管；外部加密DNS识别限制如实说明。

### T10 节点分组、选点、固定与租约准入

- Issue：[#15](https://github.com/go-laoji/honk-proxy-server/issues/15)。
- 功能：GROUP-01、GROUP-02、GROUP-03、GROUP-04、GROUP-05。
- 工作：成员过滤、固定/故障切换/延迟优选/负载均衡、候选解释、回退范围、寿命约束的会话固定；API/CLI/页面。
- 验收：不能越过固定或回退范围；UDP排除TCP-only；到期/禁用不选；已有连接不承诺迁移；入口共享选择。

### T11 规则 CRUD、模拟与一致性发布

- Issue：[#16](https://github.com/go-laoji/honk-proxy-server/issues/16)。
- 功能：RULE-01、RULE-02、RULE-03、RULE-04、RULE-05、RULE-06。
- 工作：域名/CIDR/端口/协议/入口/源地址与本机进程规则；首条命中；节点/组/直连/拒绝；引用校验、模拟、代际发布；API/CLI/页面。
- 验收：无账号条件或功能权限；模拟与实际决策一致；用户态和eBPF原子版本一致；目标不支持条件不能误显示支持。

### T12 节点管理与来源详情控制台闭环

- Issue：[#17](https://github.com/go-laoji/honk-proxy-server/issues/17)。
- 功能：NODE-01、NODE-02、NODE-03、NODE-04、NODE-06、DASH-02。
- 工作：节点分页查询、导入预览、协议能力/租约/退休状态、启停/排空/删除、来源/组/连接关联与摘要；API/CLI及共享MPA。
- 验收：URL恢复筛选；批次归属不误删；过期与健康分开；敏感数据脱敏；界面不是静态演示数据。
- 进度：已完成操作门、库存、导入、来源水位及三端接口；节点/连接的独立桌面窗口联合验收已通过，Issue #17 已关闭。见 [使用说明](node-management-usage.md)、[实际证据](validation/node-management-2026-11-02.md) 与 [连接验收](validation/connections-2026-11-02.md)。

### T13 活跃连接、流量统计与路由解释

- Issue：[#18](https://github.com/go-laoji/honk-proxy-server/issues/18)。
- 功能：CONN-01、CONN-02、CONN-03、CONN-04、DASH-03、DASH-04。
- 工作：连接生命周期、源/目标/入口/出口/命中规则、流量/异常摘要、有界列表和主动断开；API/CLI/共享页面。
- 验收：实际转发计数；不解析HTTPS正文；主动断开范围明确；重连补快照、切页取消，不无限内存增长。

### T14 后台任务、SSE 与结构化事件

- Issue：[#19](https://github.com/go-laoji/honk-proxy-server/issues/19)。
- 功能：TASK-01、TASK-02、TASK-03、TASK-04、LOG-01、LOG-02、LOG-03、LOG-04。
- 工作：统一任务ID/终态/版本、重复触发保护、跨页面恢复、事件过滤/游标重连、原生订阅取消与上游凭据脱敏；API/CLI/页面。
- 验收：客户端离开不丢后台任务；游标过期补快照；切实例不收旧事件；无项目登录或已认证操作人假设。

### T15 Linux 透明数据面产品化与恢复

- Issue：[#20](https://github.com/go-laoji/honk-proxy-server/issues/20)。
- 功能：LINUX-03、LINUX-04、LINUX-05、LINUX-06。
- 工作：本机/LAN接管范围、bypass、网卡/路由变化、热更新、启停与对象所有权；真实TCP/UDP和DNS；API/CLI/页面。
- 验收：以6.12 LTS及当前可用Linux内核为初始验证目标，完成x86_64/aarch64真实出口矩阵；失败不假报ready；清理不损伤其他网络对象；异常退出可恢复。最终最低内核要求由本项目程序与测试决定。

### T16 Desktop 本地远程档案与系统服务集成

- Issue：[#21](https://github.com/go-laoji/honk-proxy-server/issues/21)。
- 功能：INST-03、SYS-05。
- 工作：同页面管理本机/远端；持久系统服务安装/启动、系统代理原值恢复；窗口退出与服务生命周期分离，无用户认证。
- 验收：打包后导航/原生transport/目标切换；窗口关闭代理仍由系统服务运行；不以root运行WebView；远端操作不误改本机代理。

### T17 分包发行、端到端与发布门槛

- Issue：[#22](https://github.com/go-laoji/honk-proxy-server/issues/22)。
- 功能：工程基础或发行验收，无重复功能归属。
- 工作：独立headless/desktop流水线、嵌入相同web产物、文档/安装包/锁定依赖；端到端、故障注入、压测和升级回退。
- 验收：80个适用P0验收证据，真实Linux eBPF必需；供应商按用户授权的快代理契约与免费源实测，明确未验证付费账号调用；无桌面安装与18页一致；不把mock或骨架称为正式功能版本。

### T18 历史观测、诊断导出与任务增强

- Issue：[#23](https://github.com/go-laoji/honk-proxy-server/issues/23)。
- 功能：DASH-05、NODE-07、HEALTH-06、DNS-05、CONN-05、RULE-07、LOG-05、LOG-06、TASK-05。
- 工作：按预算实现历史趋势/画像/关联、规则统计导入、脱敏诊断、告警和服务明确支持的取消/重试。
- 验收：来源/时间窗口/保留期明确，结果有界可核实；不可取消阶段不伪回滚，通知状态变化限频。

### T19 SOCKS5 UDP、订阅与高级分组扩展

- Issue：[#24](https://github.com/go-laoji/honk-proxy-server/issues/24)。
- 功能：IN-07、PROV-13、GROUP-06。
- 工作：按实际需求加入UDP ASSOCIATE、标准订阅、嵌套组和高级Score。
- 验收：协议关联生命周期与来源归属正确；不替代短效API主线；嵌套循环拒绝，高级策略可解释。

### T20 备份恢复、系统体验与高级 Linux 诊断

- Issue：[#25](https://github.com/go-laoji/honk-proxy-server/issues/25)。
- 功能：CFG-07、SYS-06、LINUX-07。
- 工作：SQLite一致性备份恢复、托盘/通知/自启选项、拓扑和内核诊断。
- 验收：备份不直接复制写库；桌面自启和Daemon开机启动分离；高级数据面变更单独验收。

### T21 多实例与其他平台等储备范围

- Issue：[#26](https://github.com/go-laoji/honk-proxy-server/issues/26)。
- 功能：FUT-01、FUT-02、FUT-03、FUT-04。
- 工作：储备多实例编排、macOS/Windows透明、成本分析、外部规则/复杂评分/FakeIP；进入实施前再按各领域细分。
- 验收：不作为首版隐含范围；先明确产品语义、平台方案、真实供应商口径和兼容性后再拆实施Issue。

## 4. 首个开发切片

T01 交付独立 Rust workspace、hps/hpsd、无认证管理 API、TOML 启动配置、实际 capabilities 和状态、CLI JSON 输出以及 headless 测试。尚未接入数据面时必须返回 proxy_ready=false，不将管理接口启动成功当成代理可用。

T02 随后接管现有模板：共享页面清单、Tailwind 布局与 API transport、18 页静态资源、Daemon 嵌入与 Tauri 原生桥。业务模块尚未接入时清楚显示状态，不填造节点或成功数据。

现有未跟踪模板中，实施需要的 package/锁文件、React 入口、Tauri 配置与 Rust 入口、忽略规则会明确纳入变更；无关编辑器配置和未使用示例素材保持原状。前端新增 web/console，桌面复用 src-tauri，未使用的 src 演示素材保留原状；Cargo 的 CLI/Daemon 独立构建边界必须保持。

## 5. 开发记录

- 2026-11-01：完成 21 个子 Issue 及全部 99 项功能的唯一映射，准备开始 T01。

- 2026-11-01：T01 基础交付通过 Rust 单测、clippy、独立进程/CLI 冒烟和无 GUI 依赖检查。代理与 eBPF 尚未接入，proxy_ready=false；开始 T02。

- 2026-11-01：T02 的 18 页 MPA、共享构建清单、Daemon 嵌入及 Desktop 原生桥通过编译、4 项浏览器 E2E 和实际打包窗口验收，详见 [基础验收](validation/runtime-foundation-2026-11-01.md)。开始 T03；尚未加载真实 eBPF。

- 2026-11-01：按用户澄清修正 T03：honk 仅供能力学习，自研 Rust engine、eBPF 与 loader；T04 不再等待 honk 集成。保留真实 Linux 发布门槛，按验证切片及时提交。
- 2026-11-01：T03 第一片交付 Rust 只读环境诊断、管理 API 和 `hps linux preflight`。真实 Linux 容器识别 BTF/cgroup2、缺失的 bpffs 和能力；macOS 返回 unsupported。始终不将预检等同接管，T03 保持进行中，详见 [预检验收](validation/linux-preflight-2026-11-01.md)。
- 2026-11-01：T04 自研显式代理、资源限制及 API/CLI/共享 MPA 入口管理完成。3×3 协议矩阵、macOS/Linux 真实测试、Chromium 经过供应商出口、Desktop 实际启停均通过。按四个实现切片提交；完整证据及后续边界见 [显式代理验收](validation/explicit-proxy-2026-11-01.md)。T03/T15 的真实 eBPF 仍未完成。

- 2026-11-01：T05 配置版本、SQLite、共享草稿、热切换/排空、CLI、管理 HTTPS、离线恢复与原生导出完成。macOS/Linux 各 33 项 Rust 测试、7 项浏览器 E2E 及 Desktop 实际保存/激活/导出/回滚通过；见 [配置验收](validation/configuration-2026-11-01.md)。

- 2026-11-01：T06 来源管理、快代理签名/提取/只读诊断、通用接口、云代理 GBK 免费源、独立批次租约、动态池和安全提取完成。macOS/Linux 各 49 项 Rust 测试、8 项浏览器 E2E、Desktop 实际提取 15 条与 CLI 同任务幂等查询通过；见 [供应商验收](validation/provider-leases-2026-11-01.md)。T07 接续自动补池。

- 2026-11-01：T07 按需/维护补池、滞回水位、退休预测、统一持久预算、冷却/暂停/确认恢复、API/CLI/共享控制台完成。macOS/Linux 各 64 项 Rust 测试、9 项浏览器 E2E、独立 CLI 持久重启与 Desktop 桥/打包通过；见 [补池验收](validation/replenishment-2026-11-01.md)。T08 接续健康筛选与真实反馈。

- 2026-11-01：T08 统一健康、分维度实际探测、真实反馈、持久任务/预算、补池待检预留和 CLI/共享 MPA/Desktop 完成。macOS/Linux 各 82 项 Rust 测试、完整 10 项浏览器 E2E、实际 Desktop 探测/策略保存与 CLI 幂等/冲突验证通过；见 [健康验收](validation/health-2026-11-01.md)。T03 继续交付真实 eBPF，再推进依赖它的 T09 DNS。

- 2026-11-01：T03 自研 Rust eBPF hook/ABI/loader 完成。实际 aarch64 Linux 7.0.14 与 QEMU 启动的 6.12.115 LTS 均验证 IPv4/IPv6 连接、map 策略拒绝、卸载后恢复及程序/map/cgroup 回收；见 [真实内核验收](validation/linux-ebpf-core-2026-11-01.md)。透明转发继续由 T15 交付，继续 T09 DNS。

- 2026-11-02：T09 完成共享 DNS、TTL/负缓存、UDP/TCP/DoT/DoH、静态代理出口与实际双内核 DNS 捕获/代际/回收；见 [DNS 验收](validation/dns-2026-11-02.md)。独立桌面包已构建，CUA 仍报告锁屏，窗口验收待补，Issue 保持 open。
- 2026-11-02：T10 完成统一分组选点、严格固定/回退、会话限制及 CLI/API/共享 MPA；macOS 109、Linux 108 项测试与 clippy、13 项浏览器回归及独立桌面构建通过，见 [分组验收](validation/groups-2026-11-02.md)。窗口待解锁时继续推进 T11，不将挂起的窗口验收当成已完成。
- 2026-11-02：T11 显式首条规则、四种动作与默认选择、候选/激活分离、只读模拟、CLI/共享 MPA/native bridge 完成；macOS 122、Linux 121 项 Rust 测试、14 项浏览器回归和 clippy 通过，见 [规则验收](validation/routing-rules-2026-11-02.md)。修复按来源/策略/入口隔离补池及手动入口状态热更新；本机进程来源与内核原子代际继续与 T15 联合验收，Issue 保持 open。
