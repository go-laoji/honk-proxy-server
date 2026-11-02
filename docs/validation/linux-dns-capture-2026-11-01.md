# T09 Linux DNS 捕获核心验收切片

这是 Issue #14 的实际内核记录。共享 DNS 解析、API/CLI/MPA 与 Daemon 配置接入已实现；完整跨平台验收见 [DNS 验收](dns-2026-11-02.md)。本记录不代表完整 T15 已完成。

实际运行同一份自有 Rust 对象 `c308612ca777c19a1f61323132707f9901476cf386e2d260b2a3c11fb1867051`：

| 内核 | 环境 | IPv4 / IPv6 | 每族 redirect / restore | 错误 |
| --- | --- | --- | --- | --- |
| 7.0.14-linuxkit | 独立 privileged、无外网 Docker 网络命名空间 | UDP connected、单 socket 两 DNS peer 乱序回复、TCP | 4 / 5 | 0 |
| 6.12.115-0-virt | 签名 Alpine 内核、QEMU TCG 真正启动、仅 loopback | 同上 | 4 / 5 | 0 |

原始记录：[Linux 7.0](linux-dns-kernel7-2026-11-01.json)、[Linux 6.12](linux-dns-kernel612-2026-11-01.json)。每次均验证 8 个实际 program、5 个引用 map、4 个不重复 resolver alias；`getpeername` 返回原 DNS 地址和 53 端口。DNS 回答来自本地实际 UDP 上游，IP 不经 FakeIP 替换。

测试将父测试进程也移入自有 cgroup，向独立 53 端口发送不可解析为 DNS 的字节串，验证父进程请求完全旁路。排除条件使用 PID namespace 的 device/inode 和 namespace 内 PID，Docker 中 getpid 与 init namespace PID 不一致时仍然有效。IPv4 bypass 计数为 1。netns cookie 限制捕获在监听器的网络命名空间内。

生命周期检查包括 namespace 级独占文件锁、双栈 wildcard 监听只绑定 lo、IPv6 自有 `/112` local route 的实际存在与删除、部分加载失败回收、program ID 消失、map 经过 RCU 后返回 ENOENT、再次取得所有权、删除自有 cgroup。不会移动用户进程或替换无关路由。测试只移动它自己的父子测试进程。

工具链固定 nightly-2026-09-01、Aya eBPF 0.3、用户态 Aya 0.14、官方 bpf-linker 0.12。编译使用 BPF v3。实测发现 LLVM 22 对有返回值的 relaxed atomic add 生成非 FETCH 指令；改用 SeqCst intrinsic 后验证返回旧值、四个 resolver 分配四个 slot。关联 [LLVM 原始修复说明](https://lists.llvm.org/pipermail/llvm-commits/Week-of-Mon-20260713/1980830.html)。IPv6 context 使用逐字 volatile store，避免数组复制产生 verifier 不允许的 modified context pointer。

PID namespace helper 在此 program type 上要求支持 Linux 6.10 新增能力；本切片实际验证 6.12 LTS 和 7.0。T03 的 connect 基础检查不能单独证明这些 DNS hook 可用，最终仍以真实 verifier/attach/收发证据为准。

复现：先 `scripts/build-ebpf.sh`；在 Linux 编译 `cargo test --locked -p hps-platform --test dns_capture --no-run`。在独立、有权限且 cgroup v2 可写的 Linux namespace 中执行测试二进制：

```sh
HPS_EBPF_OBJECT=/path/hps-ebpf /path/dns_capture-TEST_HASH --exact real_dns_capture --ignored --nocapture
python3 tests/linux/ebpf/verify-dns-report.py kernel.log --kernel-prefix 7.0
```

6.12 驱动沿用 `tests/linux/ebpf/Dockerfile`，执行脚本时指定 `HPS_DNS_TEST` 为实际 musl 测试二进制，`HPS_EBPF_OBJECT` 为同一对象。脚本将其带入真实 initramfs，不将宿主内核结果当作 6.12 结果。

这里仅接管所配置 cgroup 的本机应用 DNS。网关 TC 接管、完整透明 TCP/UDP、连接路由仍属于 T15。外部 DoH/DoT 不自动解密，未知或多候选域名仍按 IP/未知处理。

## Daemon 服务级复验（2026-11-02）

最终工作区代码在两套实际内核再次通过 `hps-application` 的 `real_dns_capture`：Supervisor 启动、双栈 TCP/UDP 53、同 capture 热更新保留 8 个 program ID、关闭 capture 进入 pending_restart、撤销待重启配置、无效 ELF 和端口占用保持旧 head、shutdown 后重新取得对象所有权。每族真实重定向及回复恢复均有计数，所有阶段结束回收 program/map/自有路由/cgroup。

原始记录：[Linux 7.0 服务级](linux-dns-application-kernel7-2026-11-02.json)、[Linux 6.12 服务级](linux-dns-application-kernel612-2026-11-02.json)。复现使用 `hps-application --test dns_capture` 的实际测试二进制，并为验证器加 `--application`，不能仅用核心 loader 测试代替服务配置验收。
