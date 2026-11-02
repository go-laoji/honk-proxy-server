# macOS 网络配置验收事故与修复

2026-11-02，用户报告开发宿主 Wi-Fi 断开并导致开发中断。只读检查发现系统默认网络配置中残留本次验收的 `isolated` 测试项，确认测试越过隔离边界。用户恢复连接后，Wi-Fi 的 IPv4、IPv6 和 DNS 显示可达；本次修复不自动回写旧网络配置。

## 原因与无效证据

`proxy/macos.rs` 的 `SCPreferencesCreateWithAuthorization` 分支把传入的私有配置标识替换为 NULL，实际打开系统默认偏好。测试又通过整份 `NetworkServices` 写入构造测试数据。虽然测试以 private 标识跳过 `ApplyChanges`，`CommitChanges` 已可能修改默认持久配置；不调用 apply 不能保证不影响网络。

此前 `real_systemconfiguration_private_preferences_commit_and_restore` 输出的 pass **无效**，不得用作 #21 或发行门槛证据；该测试已删除。此前“只写临时配置”的说明不成立。真实 launchd/systemd 生命周期、代理流量、共享页面和纯事务测试是不同验证范围，不能据此推断系统代理写入验收通过。

## 立即处置

- 停止宿主 macOS 网络配置写入。系统偏好与已有备份只读复制到仓库外的本机私有目录，不提交真实偏好、网络标识或地址。
- 创建会话时以同一个配置标识传给授权与非授权路径；测试构建中不允许创建任何可写会话，提交操作也独立拒绝。
- 删除危险的 ignored 写入测试，新增“宿主测试拒绝全部写入”和“显式只读配置路径不会回落系统默认配置”的回归。
- 增加开发进程的 `HPS_FORBID_HOST_NETWORK_CHANGES=1` 运行防护。后续完整 OS 写入验收须在确认隔离的虚拟机中执行，普通临时目录不作为隔离边界。
- 继续缩小产品的写入范围，只允许修改已存在服务的 Proxies 子路径，移除整份 NetworkServices 写入能力。

宿主不再执行网络写入验收。未在隔离系统复验的实际代理写入明确保留未验证，#21 不能沿用该无效测试计为完成。
