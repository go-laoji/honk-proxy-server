# T17 分包发行与质量门槛实施计划

对应 #22。#21 已完成只读安全窗口验收并单独 finish，`develop` 已顺序合入本分支。macOS 系统代理持久写入未验收，事故前的错误测试结论已作废。禁止 push、发布 registry 或自动合入 main。

## 交付切片

1. **产物契约与证据**：整理 80 个 P0 的唯一编号、任务归属、代码/测试/已有真实验收证据、适用平台与限制；机器校验数量/重复/缺失。Linux eBPF 必须引用实际矩阵；供应商遵循用户已授权的快代理官方契约/故障测试及免费源实测，付费帐号调用明确未验证。作废的 macOS 写入测试不得计入。
2. **共同 Web 产物**：固定 Node/pnpm/Rust 工具链与已有锁文件。只构建一次 18 页 MPA，通过完整文件 SHA256 与 web_build_id 校验后作为 headless/Desktop 流水线的共同输入；构建时拒绝损坏/缺失的 manifest。无 Web 的精简 headless 构建另列能力，不冒称包含控制台。
3. **分包**：独立 headless 流水线构建 hps/hpsd、默认配置、服务模板、文档、清单与 SHA256；Linux 包附带经过验证的自研 eBPF ABI/对象及配置指引。Desktop 流水线只额外引入 Tauri/WebView，嵌入相同 Web，打包自有 hpsd 资源到可安装的持久服务位置；不因启动窗口自动运行 root 服务。实际产物与尚未运行的 CI/平台矩阵分别标记。
4. **运行/故障/负载**：在隔离环境中对安装后的二进制检验管理/TLS/代理、全 18 页、无帐号读写、端口冲突、重复实例、超限与资源释放。用受控 HTTP/CONNECT/SOCKS5/UDP/DNS 服务与有界负载记录吞吐、延迟、失败及并发峰值，不能把本机微基准冒充公网性能。
5. **升级回退**：从前一已验证构建演练持久配置/节点/任务的升级与二进制回退，覆盖不兼容 schema 拒绝与失败启动。数据库备份必须在停止写入后进行，不能复制活跃 SQLite 冒充一致性备份；T20 再实现在线备份产品。新数据库不能被旧二进制安全读取时，回退必须使用升级前停机快照。
6. **发行门槛**：汇总本轮 macOS/Linux、浏览器、原生桥、实际服务和既有 52 项真实内核证据，完善安装/升级/故障文档与可复跑脚本。若使用 changeset/版本发布流程，打本地 Git 标签；不自动推送标签或发行产物。

## 宿主边界

遵循根 AGENTS.md：宿主 Mac 的网络配置不得修改。开发/验收进程设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`，禁止运行旧的 ignored 写入测试；当前只读 SDK/事务测试不记为实际 OS 持久修改。Linux 网络测试只在自有 Docker/QEMU/cgroup/netns 中进行，并限制验证容器资源。用户原始 bundle、偏好、未跟踪文件与服务不改动。

参考 [GitHub Actions checkout](https://github.com/actions/checkout)、[artifact 传递](https://github.com/actions/upload-artifact)、[Node 配置](https://github.com/actions/setup-node) 与仓库实际的 Rust/Tauri 构建接口；工作流只拥有读取代码与写入构建 artifact 所需范围，不包含发布或远端消息动作。
