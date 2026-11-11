# 代理节点 IP 归属地

在「节点」页面查看归属地。列表和详情优先显示最近一次有效健康探测观察到的公网出口 IP 的地区。缺少出口 IP 时，服务会查询配置为公网 IP 的节点端点，界面标注「端点 IP」。域名端点需要先观察到出口 IP 才能查询。供应商提供的地区继续独立保留。

在「名称、端点或地区」中输入城市、省份、国家、国家代码或运营商，例如 `广州`。服务在分页前匹配地区。数据可能只有国家或省份，也可能查不到记录。

## 检查更新

在节点库存点击「检查地区库更新」。浏览器和 Desktop 都更新当前连接的 Daemon，包括 Desktop 内置本地服务。服务访问 ip2region 的 GitHub 仓库检查版本。已有最新版时无需重新下载；有新版本时同时验证 IPv4 和 IPv6 文件，之后立即切换。下载或验证失败时仍可查询当前版本。

更新数据保存在 Daemon 数据目录的 `ip2region/` 下，重启后继续使用。`--ephemeral` 实例在退出时丢弃本次更新。常规节点查询在服务端离线完成。

## 默认打包数据

`scripts/build-ci.py headless` 和 `scripts/build-headless.sh` 会从 [ip2region 数据目录](https://github.com/lionsoul2014/ip2region/tree/master/data) 获取最新的 `ip2region_v4.xdb` 和 `ip2region_v6.xdb`。同次 CI 或独立 Desktop 打包使用前一步 Daemon 的同一快照。直接 `pnpm tauri build` 时，Tauri 的构建钩子会拉取数据，需要可用的 `python` 命令。

可执行文件通过 `--build-info` 报告内置数据库版本。页面版本由 IPv4 和 IPv6 Git blob SHA 的前 12 位组成。单独开发或测试时也可运行 `python -B scripts/fetch-ip2region.py`，后续本地编译会读取下载结果。已有快照可用于离线源码编译。
