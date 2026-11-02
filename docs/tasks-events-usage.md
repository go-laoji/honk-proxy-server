# 后台任务与结构化事件

`hpsd` 持有已经受理的工作。关闭浏览器、切换 MPA 页面、退出 CLI 等待或关闭 Desktop 窗口，只结束客户端等待/订阅，不撤销服务已受理的配置预检、激活、提取、健康检测或诊断。正常关闭 Daemon 会停止接收工作，等待已登记工作落到可查询的状态后关闭存储。

## 任务查询与恢复

`tasks.html` 展示统一任务 ID、单调版本、类型、状态/阶段、来源记录、实际配置版本或指纹、进度与脱敏结果引用。供应商、健康、激活保留各自的原始记录引用 `provider:N`、`health:N`、`activation:N`，不把原始数字 ID 当成统一 ID。

筛选条件与所选任务写入 URL，刷新/前进/后退可以恢复。供应商和健康详情提供统一任务链接；统一详情可跳转关联事件与只读配置版本。链接带观测实例标识，服务重启后明确报实例变化；清除限制是一次显式重新查询。

任务列表按登记 ID 降序，单页 1–200 条，`before` 继续向旧记录翻页。终态最多保留 10000 条，按登记 ID 淘汰；进行中的任务另受对应工作容量限制。详情不存在可能表示已被淘汰，不能据此判断可以重放一次已付费提取。列表不携带完整诊断结果，详情按需读取。

```sh
hps --endpoint http://127.0.0.1:17900 --output json task list --kind health --limit 20
hps --endpoint http://127.0.0.1:17900 --output json task list --source-ref health:12
hps --endpoint http://127.0.0.1:17900 --output json task list --request-id prepare-example
hps --endpoint http://127.0.0.1:17900 --output json task show 42 --expected-epoch INSTANCE_UUID
hps --endpoint http://127.0.0.1:17900 --output json task wait 42 --expected-epoch INSTANCE_UUID --wait-secs 60
```

`wait` 上限 3600 秒，等待超时退出 6，任务失败退出 5，实例/版本/幂等冲突退出 7，输入错误退出 2。等待中断后仍可按原 ID 或 request_id 查询。

## 可恢复的配置提交

任务页面可以先读取当前脱敏配置，再选择“仅配置校验”或“预检并保存未激活候选”。提交前生成 request_id，并将其与实例标识放入 URL；原始 TOML 不写入 URL、任务记录或浏览器存储。相同 request_id 与相同输入返回同一任务，改变动作、期望版本、正文或备注会产生冲突。需要改动输入时显式生成新 ID。**生成候选不会激活配置**，激活继续在配置工作台审阅和执行。

```sh
hps --endpoint http://127.0.0.1:17900 --output json task submit-config ./candidate.toml --request-id prepare-example --expected-revision 1 --prepare
```

CLI 未指定 `--expected-epoch` 时先读取当前服务实例；自动化可显式传入已确认的实例 UUID。HTTP `POST /api/v1/tasks` 必须携带 `expected_epoch`、request_id 和 operation。目前异步提交支持 `configuration_validate`、`configuration_prepare`；其他已接入诊断由各自既有 API 执行，同时生成统一任务。

脱敏配置中的凭据占位遵循配置工作台已有的凭据合并规则。任务成功不表示所有检测节点可达；供应商 `outcome_unknown` 表示可能已经计费，页面与 CLI 均不自动重放购买。T14 不提供伪造的取消/重试按钮；后续 T18 只对实际允许的阶段增加能力。

## 事件快照与持续跟踪

`events.html` 按级别、模块、统一任务 ID、请求 ID、节点、供应商与起始时间筛选。先读取快照，再从最后完整游标订阅；暂停/隐藏/离开页面会关闭订阅。恢复时补取暂停期间仍保留的事件，切换筛选时清理旧详情。单页内存保留最近 500 条，界面显示最近 100 条。

```sh
hps --endpoint http://127.0.0.1:17900 --output json event list --module management --limit 100
hps --endpoint http://127.0.0.1:17900 --output json event follow --task-id 42 --reconnect-attempts 5
hps --endpoint http://127.0.0.1:17900 --output json event follow --cursor INSTANCE_UUID:123 --max-frames 100
```

HTTP 接口为 `GET /api/v1/events`、`GET /api/v1/events/stream`。SSE 帧为 snapshot/events/checkpoint/reset/unavailable，ID 为完整游标。CLI follow 使用逐行 JSON，额外提供 transport 和 end 控制帧，Ctrl+C 会释放订阅。正常 JSON 请求超时不截断长期 SSE 订阅；SSE 有连接/读取期限、心跳和帧大小限制。

服务保留最近 10000 条事件，每批最多 200 条；游标由每次启动的新 UUID 和单调序号组成。无匹配记录时 checkpoint 仍推进扫描位置。旧实例、超前游标或超出保留窗口会明确 reset 并补最近快照；窗口外的历史无法恢复，不能称为完整历史。重连仅携带最后完整且通过验证的游标，协议错误停止订阅并显示原因。

浏览器使用同源 EventSource；Desktop 通过 Rust 固定接口和带确认的 Channel，最多 4 个原生工作器，每窗口一个当前订阅。页面卸载、窗口销毁或目标切换取消旧连接；目标/文档代际与订阅标识拒绝晚到事件。每帧等待对应 ACK，最长 15 秒，避免原生 IPC 队列无限积压。服务端最多 16 个事件订阅、16 个通用工作槽位，超量明确返回容量错误。

## 来源与信息边界

管理事件记录实际 TCP 对端、固定动作、资源和结果；忽略客户端伪造的转发来源头。当前项目没有登录、账号、权限体系，所以 TCP 对端不能当作已认证操作人。后台状态事件可以没有网络对端。

事件不保存业务正文、完整 URL/查询参数、认证头、上游响应原文、配置正文或备注。任务仅保留有限的安全诊断摘要或已有资源引用；输入指纹用于幂等比较。配置、上游密钥和业务流量仍按现有功能各自处理，不能从事件缺少这些内容推断某操作没有发生。
