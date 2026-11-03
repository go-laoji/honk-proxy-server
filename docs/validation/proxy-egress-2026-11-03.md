# 真实代理出口验收（Issue #29）

2026-11-03 08:25（Asia/Shanghai），用户提供的长效、用户名密码认证 SOCKS5 节点通过验收。curl 直接使用节点与通过 honk-proxy-server 的 HTTP CONNECT、SOCKS5、SOCKS5h 入口访问同一国内 HTTPS 接口，出口均为 **106.38.56.186**，与本机 IPv4 直连出口不同。没有复现该链路的程序缺陷，未修改应用代码。

- 任务：[Issue #29](https://github.com/go-laoji/honk-proxy-server/issues/29)。
- 国内目标：[IPIP 出口回显](https://myip.ipip.net/)，响应须包含可解析的有效出口 IP；只收到 HTTP 200 不算验收通过。
- curl 8.7.1；所有请求禁用 `.curlrc`，明确设置 `--proxy` / `--noproxy`、超时和 IPv4。目标 TLS 使用默认信任校验，无 `-k`。
- 结构化证据：[proxy-egress-2026-11-03.json](proxy-egress-2026-11-03.json)。供应商地址、端口、用户名、密码和本机直连出口均未收入报告。

## 结果

| 访问路径 | HTTP 状态 | CONNECT 状态 | TLS 校验结果 | 出口 IP |
| --- | --- | --- | --- | --- |
| curl → SOCKS5 节点，客户端解析目标 | 200 | 不适用 | 0，通过 | 106.38.56.186 |
| curl → SOCKS5h 节点，代理解析目标 | 200 | 不适用 | 0，通过 | 106.38.56.186 |
| curl → hps HTTP CONNECT → SOCKS5 节点 | 200 | 200 | 0，通过 | 106.38.56.186 |
| curl → hps SOCKS5 → SOCKS5 节点 | 200 | 不适用 | 0，通过 | 106.38.56.186 |
| curl → hps SOCKS5h → SOCKS5 节点 | 200 | 不适用 | 0，通过 | 106.38.56.186 |

五项代理请求 curl 退出码均为 0。直接请求的 TCP peer 是用户提供的节点，经 hps 请求的 peer 是本机隔离入口；两者分别核对实际地址，不仅依赖 IP 回显。直连基线也通过 HTTPS/TLS 校验，原始基线仅存本机私有临时记录。

hps 开启真实健康检测，探测同一个 HTTPS 目标：节点 `eligible=true`、`verdict=healthy`、`authenticated=true`、`tunnel_established=true`、`target_ok=true`，目标 HTTP 200。`/readyz` 的 `explicit_ready=true`。

三条业务连接均记录选中 `long-lived-socks5`，上游协议 `socks5`，端点与用户提供的节点相符。连接统计为 started/opened/closed 各 3、failed/rejected 各 0、活动连接 0；上行 1,566 字节，下行 14,323 字节，逐条 `error_code=null`。使用 `state=all` 查询已结束连接；默认查询仅返回活动连接。

## 运行代码与隔离

- 使用此前本机实际构建的 release `hpsd`，源码提交 `c79e07dc626f37fb41d78b83fa340648afe7968f`，二进制 SHA-256 为 `6fcd7db562d1b3cffd0585b547b76f513df097b78e768fba3305c45032dd9055`。
- 验收分支提交为 `f364a0d387d53f6872cda19f63ced9cd728a3a9f`。已核对上述两提交的 `crates/`、`apps/`、`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml`，无差异；这是复用同一运行时代码的构建产物，本轮未重编译。
- 使用全新 `--ephemeral` 实例，仅监听 `127.0.0.1` 专用端口；只有该静态上游，未配置免费供应商或直连规则。检测服务实例名，避免误连原有服务。
- 测试进程设置 `HPS_FORBID_HOST_NETWORK_CHANGES=1`；未写宿主 Wi-Fi、DNS、路由、系统代理、原服务数据库或 Desktop 偏好。
- 私有节点配置只在仓库外临时目录，目录权限 0700，配置权限 0600；curl 通过标准输入读取代理凭据，没有将其加入命令行参数、报告、提交或 Issue。
- 两次隔离实例均在验证结束后退出，最后一次业务连接已全部关闭；原有服务未停止或重启。

## 复现方式

在仓库外创建权限 0600 的 curl 配置，填入节点的 `proxy-user`，并按 `configs/hps.proxy.example.toml` 创建私有 hps 配置：仅启用用户提供的 `socks5` 静态上游，在 `[proxy.upstreams.credentials]` 中填入凭据，设置专用回环管理/HTTP/SOCKS5 监听。健康 profile 指向同一个 `https://myip.ipip.net/`、IPv4/TCP、expected_status=200，保留健康准入。

以下变量须设为本机对应路径、节点地址及入口端口；`CURL_PROXY_AUTH` 只包含 `proxy-user`，不包含其他 curl 选项。不得将实际凭据写入仓库。

```sh
# 直连基线，原始回显不要加入版本控制。
curl --disable --ipv4 --proxy '' --noproxy '*' \
  --connect-timeout 8 --max-time 20 https://myip.ipip.net/

# 先直接验证节点。将 socks5 改为 socks5h 可验证代理侧目标解析。
curl --disable --config "$CURL_PROXY_AUTH" --ipv4 \
  --proxy "socks5://$PROXY_HOST:$PROXY_PORT" --noproxy '' \
  --connect-timeout 8 --max-time 20 https://myip.ipip.net/

HPS_FORBID_HOST_NETWORK_CHANGES=1 "$HPSD" \
  --ephemeral --config "$PRIVATE_HPS_CONFIG"

# 在另一终端确认 /api/v1/health 已 healthy、eligible，随后运行：
curl --disable --ipv4 --proxy "http://127.0.0.1:$HPS_HTTP_PORT" \
  --noproxy '' --connect-timeout 8 --max-time 20 https://myip.ipip.net/
curl --disable --ipv4 --proxy "socks5://127.0.0.1:$HPS_SOCKS_PORT" \
  --noproxy '' --connect-timeout 8 --max-time 20 https://myip.ipip.net/
curl --disable --ipv4 --proxy "socks5h://127.0.0.1:$HPS_SOCKS_PORT" \
  --noproxy '' --connect-timeout 8 --max-time 20 https://myip.ipip.net/
```

同时记录 curl 退出码、HTTP 状态、`ssl_verify_result`、`remote_ip` 和 HTTP 入口的 `http_connect`；核对 `/api/v1/connections?state=all&limit=20` 的选点及双向字节。测试后只终止自己启动的隔离实例。

## 此前免费来源的结论与边界

暂停前最后一轮真实提取 8 条 `ip3366.net/free/?stype=1` 租约，使用 no-cache/Pragma 和查询参数绕过缓存。发布后 0.519～0.532 秒开始并发 curl，约 5.511 秒完成；0/8 验证成功，健康可准入数为 0，hps CONNECT 返回 503。此前样本从当前宿主访问未通过，重新下载页面不保证供应商列表中的节点仍可用；本地 TTL 不是供应商寿命保证。

长效节点证明当前 SOCKS5 认证、健康准入、HTTP CONNECT / SOCKS5 入站及真实 HTTPS 转发链路可用。本次没有重新探测免费列表，也不据此宣称免费来源、所有代理协议、IPv6、UDP 或系统透明接管已通过。当前证据没有复现需要修复的程序问题。
