# 配置版本使用说明

CLI、浏览器与 Desktop 共用 Daemon 的 SQLite 配置版本。接口不设置项目登录、账号或权限验证。数据库会保留实际供应商凭据，普通 API、差异与导出均脱敏。

## 启动和持久化

```sh
hpsd --config hps.toml --data-dir /srv/hps/data
hps config workspace
```

TOML 只用于新目录初始化；已有数据库的期望版本优先。实例锁防止两个 Daemon 同时使用同一目录。新目录权限为 0700，数据库与锁文件为 0600。`--ephemeral` 使用内存数据库，关闭即丢失，适合验证环境。

`--listen` 与 `--name` 是显式启动覆盖，工作台会显示。启动覆盖会继续作用于配置激活，需移除启动参数才能使用对应持久化值。TLS 路径在首次 TOML 加载时相对 TOML 目录解析；API 导入则相对数据目录解析。

## 编辑、校验与激活

控制台“配置工作台”展示期望版本、实际运行版本、共享草稿版本和仍有连接的旧代际。保存草稿不激活。校验通过后可保存不可变版本，再显式激活。输入错误显示字段与可用的行列信息，不回显敏感源行。

```sh
hps config validate candidate.toml --expected-revision 1
hps config save-draft candidate.toml --expected-revision 1 --draft-version 1
hps config import candidate.toml --expected-revision 1 --note '调整入口资源限制'
# 使用 import 返回的 revision，例如 2
hps config activate 2 --expected-revision 1
hps config export --revision 2 --path exported.toml
```

所有写入均检查期望版本；共享草稿还检查独立草稿版本。其他页面提交后，旧页面返回 HTTP 409，CLI 退出码为 7，需要重新读取。校验失败退出码为 5，不会发布配置。导出路径必须是新文件，CLI 不覆盖已有文件。

`[REDACTED]` 表示同 ID 已有凭据引用。普通文件导入默认引用当前期望配置；从共享草稿继续编辑时，CLI 显式传 `--use-draft-credentials --draft-version N`，浏览器工作台自动维护该来源。找不到原值或草稿版本过期时拒绝保存，避免猜测凭据。新来源或轮换凭据应填写实际新值。导出的脱敏文件不包含独立恢复所需的秘密。

候选监听、证书与配置先准备，通过后才提交期望版本。绑定失败或证书错误保留原运行代际；相同监听复用套接字，旧连接按原配置排空，新连接使用新版本。连接上限跨代际计算。激活客户端断开不取消已经受理的后台切换，结果可从激活历史查询。

管理地址、管理证书或日志过滤变化会显示 `pending_restart`，期望配置与实际运行配置分别展示。也可使用 `--defer-until-restart` 暂存整个激活。系统不会把尚未绑定的管理地址报告为已运行。

## 回滚、恢复和 HTTPS

```sh
hps config rollback 1 --expected-revision 2
hps --endpoint https://localhost:17900 --ca-cert management-ca.pem status
# 管理监听配置错误导致无法启动时，停止原进程后离线选择已运行成功的版本
hpsd --data-dir /srv/hps/data --recover-revision 1
```

回滚仅允许历史上实际应用成功的版本，仍须经过当前证书、端口和引用校验。离线恢复适用于管理端口错误导致 API 不可达的场景，不跳过版本校验。管理 HTTPS 使用 `[management_tls]` 的 `certificate`、`private_key`，CLI 的自定义 CA 仍验证证书链和名称，没有忽略 TLS 错误的开关。Desktop 自定义 CA 配置由桌面连接档案任务继续交付。

默认保留最近 200 个配置版本，额外保护期望、实际运行、草稿基线和仍有连接的流量代际。`retention.config_versions` 可配置为 10–10000。`activity_days`、`task_days` 随对应模块启用，不表示当前已有历史事件和统一任务功能。
