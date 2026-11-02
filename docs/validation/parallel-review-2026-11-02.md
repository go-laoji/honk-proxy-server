# #23–#26 并行交叉审查记录

- 日期：2026-11-02（Asia/Shanghai）。
- 定位：本轮 Epic 交付过程记录，记录相邻功能的审查发现、修复及针对性证据；不新增产品功能或实施 Issue，不修改中央进度。
- 记录分支：`feature/future-platform-contracts`，记录前 tip `88f4685`。T21 原有[范围与兼容契约](../future-platform-contracts.md)、计划和验收记录保持不变。
- 审查方式：T21 agent 在各功能专用 worktree 只读查看源码、提交 diff、回归测试代码及实现 agent 的验收记录；不修改其他 worktree，不启动 Daemon/Desktop，不执行持久恢复或重型构建。

## 1. 结论与证据归属

下列问题已经实现负责人修复并提供 focused 回归，T21 agent 复核了修复与测试覆盖的问题路径。**测试执行者是 root 或对应实现 agent，T21 agent 没有亲自重跑这些运行测试。** 本记录中的“通过”均按各实现负责人的执行结果及其仓库记录转录，源码复核与测试执行分开列出。

本记录初版生成时，root 正在进行最终 macOS/Linux 组合验收，初版不将尚未结束的全量测试、Linux 内核/eBPF 矩阵或平台打包计算为已通过。后续第 4.5 节仅引用 root 已归档的最终报告，仍不代表 T21 agent 亲自执行。各分支历史测试数量、这里的 focused 数量及最终报告不能直接相加，它们可能重跑相同用例。

| 功能 | 审查起点 | 已复核修复 |
| --- | --- | --- |
| #23 / T18 诊断与任务 | root 的 `011149b` 运行控制/告警/导出路径 | `6318c2c`：持久终态与采集结束分离 |
| #24 / T19 协议、订阅与分组 | 合并 #23 后的 `c2e87f0` | `dced51a`：解析预算；`67adf99`：订阅代际、来源寿命与嵌套查找 |
| #25 / T20 备份与恢复 | backend `646e4d6`；后续 root 恢复审查 | `54c6117`：WAL 快照/配置摘要；`79a9819`：启动状态/响应预算；`862ddad`：输入封存/精确 schema |
| #26 / T21 储备契约 | `423c41c` 基线 | `113b619` / `88f4685`：范围计划与契约；本次只增加本文 |

## 2. #23：落库失败控制项不能随重试缓存淘汰

**发现。** `crates/application/src/diagnostics.rs` 原先只按采集 `finished` 清理超过 600 秒的控制项或淘汰已完成项。终态写入失败时，数据库任务仍为 queued/running，但采集端已经设置 `finished`；过期或 128 项缓存压力可能移除其控制项。存储恢复后，该任务失去取消能力、持久化失败提示也消失，并继续占用非终态任务额度直到进程重启。

**修复。** `6318c2c` 增加 `durable_terminal` 原子标志。正常收尾或版本化取消取得持久终态后，才允许按重试输入 TTL/缓存压力淘汰；采集结束本身不等于数据库已经收尾。取消成功会更新时间，继续使用既有 epoch/version 冲突语义，不扩大可取消范围或重放采购/配置发布。

**只读复核。** 核对了 `Runtime::prune` / `insert`、`start_work` / `finish_work` 错误分支、`cancel_task` 与状态提示，及新增回归 `failed_terminal_write_retains_cancellation_past_retry_ttl_and_cache_pressure`。

**执行证据。** root 报告最终 diagnostic focused **3 项通过**及对应 Clippy 通过；见 `docs/validation/observability-2026-11-02.md` / `.json`。新增回归对真实临时 SQLite 创建 `BEFORE UPDATE` trigger，使指定任务的 completed 写入失败；撤销故障后模拟 TTL 过期和 128 项控制缓存压力，确认仍可取消，持久收尾后才可正常淘汰。它不是只给内存控制项设置失败标记的测试。

## 3. #24：代际、寿命、解析与嵌套成本

### 3.1 订阅必须在接流前绑定本代际快照

**发现。** 原 `Engine::prepare` 共享旧 `DynamicPool`，而订阅 fingerprint 只包含来源 ID、URL 和格式，不包含 snapshot。新 Engine 开始接流、发布 capture 或成为 current 后，才由后置 `refresh_pool` 替换订阅节点；窗口内新 revision 可能选择已被新快照移除的旧节点。附带的 lease 查询失败还可能让已经发布的订阅更新返回错误，与“失败保留旧快照”的约定冲突。

**修复。** `67adf99` 将订阅节点移到每个候选 Engine 的 `OnceLock<Arc<Vec<Arc<Upstream>>>>`。`prepare_proxy_engine` 在接流/可见前完成节点、资源与 TLS 预备；该快照不从 previous Engine 复制，不受后续 lease refresh 修改，且拒绝第二次设置。共享池只保留短效租约。未修改短效来源的激活跳过无关后置 lease reload，订阅发布不再依赖这个附加查询。

**只读复核。** 核对准备顺序、OnceLock 只写一次、previous 共享字段、来源过滤、probe/selection 同时读取共享 lease 与本代际订阅快照；核对成功/失败及后置查询的回归代码。候选准备时可见的内容与旧代际内容分别固定，不以 API 返回后的最终列表代替窗口验证。

### 3.2 来源最小寿命须读取本代际策略

**相邻审计补充。** 实现 agent 在修复共享池时确认：供应商 fingerprint 有意排除 `min_remaining_secs` / refill 策略，但节点缓存的 minimum 可能来自旧策略。共享 lease 池尚未刷新时，新代际会短暂沿用旧的退休/准入阈值。该项由实现 agent 扩展相邻审计定位，T21 agent 复核了最终修复，不将其记为自己独立复现。

**修复与复核。** `67adf99` 在每个 Dialer 保存 `source_lifetimes`，预备时取该代际的来源 minimum 与启用 refill 时的 `retire_before_secs`；统一用于选择、最终 admission、ProbePlan 及会话寿命预算。核对 `minimum_lifetime` 的各调用点，旧代际不被新代际策略写穿。

**针对性回归。** `source_lifetime_policy_is_generation_local_before_shared_lease_pool_refresh` 在相同来源 fingerprint、共享同一 lease Arc、未刷新池时，确认旧阈值 0 可选，新阈值 300 立即拒绝；ProbePlan 返回 300，实际 dial 返回 `NoUpstream`。

### 3.3 YAML 正文上限不能替代展开预算

**发现。** Clash 解析原先完整反序列化 `Vec<ClashProxy>` 后才检查数量及字段长度。平铺 YAML 别名能反复复制一个大型 scalar，使不足 1 MiB 的输入在拒绝前分配大量内存。锁定的 `serde_yaml_ng 0.10.0` 虽有递归/alias 跳转限制，其 `events.len() * 100` 跳转预算不能限制这种复制字节量。另将远端 GET 后的同步解析直接留在异步工作线程，也不符合有界工作归属。

**修复。** `dced51a` 在 serde/libyaml 解析前拒绝 anchors/aliases，限定缩进/flow 深度 32；序列使用有界 visitor，超过 10000 条拒绝。引号内的 `&` / `*` 和注释按普通内容处理。GET 收完有界正文后，将 owned 配置和文本送入 `spawn_blocking`，外层 tracked 工作与 provider permit 保留到解析结束。现有来源节点数、1 MiB 正文与配置持久预算继续生效。

**只读复核。** 查看锁定依赖的 alias/string 反序列化实现、前置扫描调用顺序、有界序列、阻塞任务归属与恒定错误；查看小型 alias、深度 33、10001 条、引号/注释及敏感字符串回归。没有运行大内存攻击样例，也没有用先展开为 YAML Value 的预解析规避问题。

### 3.4 嵌套组避免逐候选扫描大成员表

**发现。** `expand_group` 原先对每个候选执行 `group.members.contains` 等线性查找。合法的 10000 个成员与大节点池会形成候选数乘成员数的工作量；4096 次图递归预算本身没有约束这项代价。

**修复与复核。** `67adf99` 预编译组索引、members/providers/fallback 以及节点/来源标签集合，嵌套展开使用 HashSet 查找。保留父组候选并集、子组过滤/固定/fallback 边界、父组最终策略/会话和既有环/深度预算。源码复核确认去除了所报告的大成员表线性扫描；这不是全规模性能压测结论。

### 3.5 实现 agent 的 focused 结果

依据 #24 agent 报告及 `docs/validation/protocol-subscriptions-2026-11-02.md`，修复后分别执行并通过：

- provider parser **3 项**，涵盖新增 alias/深度/条数预算；准备/旧新订阅隔离 unit **1 项**。
- subscription Application integration **4 项**，包含真实临时 SQLite `ALTER TABLE` 故障令 lease 查询失败，订阅仍返回 applied 且与持久 head 一致；故障表随后在测试内恢复。
- 来源寿命 admission unit **1 项**；selection **5 项**；真实回环 SOCKS5 UDP ingress **5 项**；typed subscription API **1 项**。
- 最终 Clippy / fmt 检查通过；修复后的独立端口浏览器综合验收 **1 项**通过。

上述是不同轮次的 focused 证据，不汇总成新的 workspace 总数。UDP 来源 IP/端口、FRAG、控制 TCP 生命周期、目标额度与规则/真实数据报路径，以及嵌套候选和可解释 Score 的已查范围，未发现其他具体问题；本审查不扩展其外部客户端、LAN、Windows 或真实内核覆盖声明。

## 4. #25：恢复语义与整个下载生命周期的预算

### 4.1 内部配置摘要和启动单例也必须校验

**发现。** 原校验只解析 desired document，没有验证 `config_revisions.checksum`。修改内部 checksum 为错误的 64 字符值，再重新计算备份文件 SHA，仍可能通过 inspect/restore，但正常 `Store::read_document` 会拒绝启动。相同结构检查还遗漏启动必需的 `config_draft` 单例与语义：缺行或合法 JSON 中含未知配置字段，仍可能在发布后令 workspace 读取失败。

**修复。** `54c6117` 按原始 document 校验每条保留 revision 的 SHA 并验证配置语义；`79a9819` 补 head/draft/provider_clock/event_context 的启动不变量、draft 语义和持久 lease/probe JSON 可解析性。文件摘要、schema 结构/版本、quick_check 与 foreign_key_check 继续独立验证，不能用其中一项代替其他项。

**只读复核。** 查看校验均发生在目标库发布前，损坏输入无法进入替换路径；查看错误 checksum、缺单例、未知 draft、缺 schema 原库不变及旧 v1 保持版本、正常迁移的测试代码。

### 4.2 一个复制 worker 不等于一个在途下载

**发现。** 原 semaphore 在生成 Snapshot 返回时释放，API 随后把完整最多 64 MiB 的 Vec 放入响应 Body。前一个慢客户端未消费完时，后续请求仍能生成另一份完整缓冲，复制并发限制无法约束在途备份响应的总内存。

**修复。** `79a9819` 将 `OwnedSemaphorePermit` 放入 Snapshot，响应 stream 持有 Snapshot 直到消费结束或丢弃；逐个复制独立 64 KiB 块，避免最后一个小帧继续引用整份大分配。Native 保存等待也设置每进程单份快照额度。

**只读复核。** 查看许可归属、Body 捕获和 drop 生命周期，以及未消费第一响应时第二请求 429、drop 第一响应后第三请求 200 的 focused 回归。

### 4.3 WAL、原库与执行证据

实现 agent 在 `54c6117` 进一步固定源库 WAL 只读事务快照，避免逐页复制被高频外部写入反复重启，并增加 2 MiB 数据下持续写入可前进的一致性测试。T21 agent 只读复核了此修复及独立只读连接、逐页/期限/文件边界。

离线 restore 的其余已查范围包括与 Store 相同的 `daemon.lock`、checkpoint 忙拒绝、原库硬链接保留、候选原子 rename、目录同步失败专用结果、schema 上限及私有新文件权限；该轮 T21 只读审查未报告其他具体问题。随后 root 又实际复现并修复了第 4.4 节的问题，因此该轮结论不代表恢复输入和 schema 校验已经没有遗漏，也不是对任意文件系统/断电/外部非协作写入的全面保证。

#25 agent 报告 `cargo test -p hps-api -p hps-storage backup -- --nocapture` **6 项通过（Storage 4、API 2）**，见 `docs/validation/backup-desktop-diagnostics-2026-11-02.md` / `.json`；包含上述 WAL、校验、临时目标库原件保留、旧 schema 及响应额度回归。恢复测试由 #25 agent 在临时测试数据库执行，T21 agent 未执行恢复。其单独的 CLI/Desktop/Linux 采集证据保留在原记录中，本文不将其转换为 root 的最终组合验收。

### 4.4 Root 补充复现：输入路径替换与 schema 对象遗漏

**发现与归属。** 以下问题由 **root 实际复现、实现修复并执行验证**，提交为 `862ddad`；T21 agent 只读复核该提交的源码/测试、更新后的 `docs/backup-desktop-diagnostics-usage.md` 和最终报告，没有亲自运行路径替换攻击、恢复或组合验收。

- **原路径 A→B→A。** 旧路径多次重新打开输入，前后摘要检查均可能读到 A，而中间复制读到 B；前后再次核对原路径不能证明复制的内容就是获准的字节。修复将 `capture` 的单次有界读取结果与 SHA 绑定，写入 UUID 命名的 0700 私有目录和 0600 sealed 文件；后续 schema 校验及 Backup API 只读同一封存内容，不再打开调用者的原路径。成功/失败都清理封存对象。
- **额外 schema 对象。** 旧校验只确认预期对象存在，未拒绝额外 trigger/table/view/index；额外触发器可以在恢复后的正常 SQL 操作中改变数据库。修复先校验 `schema_migrations` 自身的对象形状，再对该版本全部非内部对象的 `name/type/tbl_name/sql` 排序精确比较，数量或内容不一致均拒绝。
- **`LIKE` 的下划线通配符。** `name NOT LIKE 'sqlite_%'` 中 `_` 匹配任意单字符，可能将 `sqliteXpoison` 这样的用户对象误当内部对象排除。修复使用 `name NOT GLOB 'sqlite_*'`，只排除字面 `sqlite_` 前缀；回归专门覆盖该绕过名称。

**只读复核的回归。** `captured_bytes_bind_digest_schema_and_restore_after_source_path_replacement` 在捕获 A 后把原路径替换为 B，确认 inspect 使用的封存字节仍为 A、恢复出的配置为 A、原路径仍保持 B，并核对 0700/0600 权限与封存清理；另确认先捕获的非法内容不能被随后替换成合法 A 救回。该回归验证修复后的同一捕获链路，不将它描述为 T21 agent 亲自重现旧 A→B→A 攻击。`extra_objects_including_literal_prefix_bypass_cannot_poison_restored_schema` 覆盖普通与 `sqliteXpoison` trigger、额外表/视图/索引：inspect/restore 均拒绝，目标原库和注入后的源文件字节均保持不变。

使用说明同时澄清：30 秒期限只覆盖逐页复制阶段，源捕获、后续校验/落盘和传输另受文件大小与并发预算约束；不再把它描述为整个备份/恢复请求的总期限。

### 4.5 Root 最终报告引用，不重新加总

最终编译源码为 `862ddad624403e6f81bc994a7fe262ac04b7fb01`。T21 agent 只读查看了以下报告；执行结果归 root 及报告列明的实现负责人，本文不重跑、不合并成新的测试总数，也不把历史 focused 数量再次相加。

| 报告 | 报告中的结果及边界 |
| --- | --- |
| `docs/validation/backlog-macos-2026-11-02.md` / `.json` | 最终 Mac workspace **227 passed**、Native **11 passed**，各自 0 failed/ignored；最终持久库浏览器 **3 项**单列。较早 `695e116` 的全量浏览器 25 项保留其原源码归属，不改记为最终源码全量。 |
| `docs/validation/backlog-linux-2026-11-02.md` | ARM 容器普通 workspace **234 passed / 30 ignored**；x86 仿真 userland **233 passed / 30 ignored / 1 条运行时 skip**。原 socket 用例 errno 92 失败保持记录；同一未剥离 ELF 在两套真实 x86 guest 内分别精确通过，该补验不并入普通 workspace 或标准 52 项。 |
| `docs/validation/backlog-kernel-2026-11-02/acceptance.json` | aarch64/x86_64 × 6.12.115/7.2.14 的标准真实内核案例 **52 项**，新增 live diagnostics 证据 **12 条**；诊断断言嵌于既有路径，不能将两者加成另一组案例数。socket 差异补验和发行 Daemon 额外案例均明确不计入该 52 项。 |

报告资料提交为 `34acb5d438994f710ac698da3dfd85d2bb75de24`。报告保留 Mac 可见 GUI、真实 OS 通知/登录自启、宿主网络持久写入、云签名/公证和付费供应商等限制；本次文档追加不将这些未验收项变成通过，也不执行任何宿主网络写入。

## 5. 固定修复来源与后续边界

| 短 SHA | 完整提交 |
| --- | --- |
| `6318c2c` | `6318c2ca9488c282c19baab5d3dbf423cb90c361` |
| `dced51a` | `dced51a34b353aafeffd9e45edfddcbd649a83be` |
| `67adf99` | `67adf993906b3cd6149a74c5b01d2677ee71fdd6` |
| `54c6117` | `54c61175b2d21d99a1d911d1bf7278c1d85ea057` |
| `79a9819` | `79a981901efd9a3c0ea34ebdc6bdf1be6423dc80` |
| `862ddad` | `862ddad624403e6f81bc994a7fe262ac04b7fb01` |

代码与测试可在共享 Git 对象库按上述修复提交用 `git show <SHA>:<仓库路径>` 复核。验收记录的资料提交分别为 #23 `c6f0755e2cc6065fe69a266948a552fd508d31fd`、#24 `67adf993906b3cd6149a74c5b01d2677ee71fdd6`、#25 `1f4be37388fbad0fca517de109778dfe9a12c7bf`；记录提交不代替对应代码修复 SHA。这样不要求本记录分支提前合入其他功能。本文只增加过程记录，不改变 #26 的未来能力边界，也不代替各 Issue 的实际结果回复。

本次记录检查：Markdown 相对链接存在、提交对象存在、无意外不可见格式字符、`git diff --check` / 暂存差异检查；只暂存并提交本文。未改动 root/其他分支、T21 原三文档或中央进度；未 push、feature finish 或关闭 Issue。没有宿主 Mac 网络写入、旧 ignored 网络测试、用户原服务/Bundle 覆盖，也没有在本文记录尚未完成的最终全量结果。
