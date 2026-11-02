# T18 / #23 验收

实现来源：`e6981cc`，附 [机器摘要](observability-2026-11-02.json) 和 [使用说明](../observability-usage.md)。代码分片提交覆盖真实趋势/关联/命中、健康历史与可选出口 IP、DNS TTL 诊断、版本化规则导入导出、白名单诊断后台工作、epoch/version 任务控制及共享状态告警。

- macOS workspace：198 passed / 0 failed / 0 ignored。其后审查修复诊断终态落库失败控制项淘汰，最终 focused diagnostic 3 passed；真实 SQLite trigger 注入终态写失败，恢复存储后即使经过重试 TTL 和128项缓存压力仍可取消，进入持久终态后才按正常 TTL 淘汰。不是仅给控制项手动置失败标志的测试。
- Native 9 passed；workspace、最终 diagnostic 与 Native Clippy 均 `-D warnings` 通过，Rust fmt、TypeScript、Prettier 和共享18页构建通过。
- Chromium 22 passed：全量共享控制台、真实代理链及任务恢复；新增真实无出口异常→入口停止恢复→设置候选→应用、诊断跨页面回查/JSON下载/脱敏和390px布局。
- 独立临时 Daemon + 编译 CLI 实测：diagnostic start --wait、task list 按请求ID、controls、show同持久结果、alerts同epoch；自有进程已退出，没有系统服务和宿主网络写入。
- 健康 plain IP 捕获包含真实代理链上的 IPv4/IPv6、非法/超长/慢响应边界；额外画像失败不推翻已证明的代理健康。规则导入版本/hash/CAS/引用/1MiB边界和候选未激活均有实际 API/浏览器验证。

趋势/通知在运行期内存有界，健康历史持久化，文档与页面分别标明来源/窗口/保留期；地域未配置时未知。只读工作重试600秒/128项，未持久收尾失败项保留取消能力；供应商购买和配置发布不可取消，不伪造回滚。系统通知由T20接入同一事实流。本项没有重新运行 Linux 内核矩阵，也不改写既有真实矩阵或把忽略案例算通过；T19/T20集成再验证。Native文件保存对话框未做可见窗口验收；macOS系统网络写入与付费供应商实时验收仍保持原限制。
