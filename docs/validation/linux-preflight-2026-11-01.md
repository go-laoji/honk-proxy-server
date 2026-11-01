# Linux 只读预检验收

- 日期：2026-11-01（Asia/Shanghai）。
- 任务：T03 [#8](https://github.com/go-laoji/honk-proxy-server/issues/8) 的第一片交付，任务整体保持进行中。
- 实现：`crates/platform`，经 `GET /api/v1/linux/preflight` 和 `hps linux preflight` 查询目标 Daemon。
- honk 只用于能力学习，本项目未引入其 crate、二进制、eBPF 对象或 fork。

## 实际结果

macOS aarch64 的 CLI 通过真实 HTTP 调用本机 Daemon，返回 `platform=macos`、`status=unsupported`、`scope=read_only_prerequisites`、`datapath_ready=false`。访问无需登录或凭据。

同一 Rust 模块在普通 Docker Linux 容器内实际编译运行。镜像为官方 `rust:1.98.1-alpine`，拉取时 digest 为 `sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f`。仓库只读挂载，未使用特权参数，临时容器运行后自动删除。

| 检查 | 实际结果 | 含义 |
| --- | --- | --- |
| 内核 | `7.0.14-linuxkit`，aarch64 | 成功读取，不据此宣布满足全部程序需求 |
| BTF | pass | 默认 raw BTF 文件头可读 |
| cgroup2 | pass | 默认路径实际挂载 cgroup2 |
| bpffs | fail | 默认 pin 路径没有可写 bpffs 挂载，目录存在不算通过 |
| 有效 UID | root | 容器 root 不代表具备 BPF/网络能力 |
| BPF/网络能力 | fail | 该普通容器不满足候选 TC 路径的基础能力检查 |
| 内核选项 | unknown | `/boot/config-*` 不可读，不推断为通过 |
| 实际程序加载 | unknown | 只读预检没有加载或附加 BPF |
| 数据面 | `datapath_ready=false` | 没有真实代理出口证明 |

## 可复现命令

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --locked -p hps-daemon -p hps-cli
./target/debug/hps --output json linux preflight

docker run --rm \
  --mount "type=bind,source=$PWD,target=/work,readonly" \
  --workdir /work \
  --env CARGO_TARGET_DIR=/tmp/hps-target \
  rust:1.98.1-alpine \
  cargo run --locked -p hps-platform --example preflight
```

Rust 工作区共 6 项测试通过，其中新增 3 项预检回归覆盖：容器 root/较新内核/目录不能推出 ready、不能直接继承参考项目的内核下限、缺少证据不得报告 pass。真实 HTTP 测试覆盖新接口。格式、clippy、CLI 实际调用、Desktop bridge 测试与 clippy，以及 4 项浏览器端到端检查通过。

## 后续验收

当前没有自研 eBPF 程序、公共 ABI、loader、map 交互或附加/回收实现；没有进行真实透明 TCP/UDP、原始目标或出口验证。程序加载还受 seccomp、LSM、verifier 和实际 hook 等条件影响。预检仅观测默认路径，不执行挂载或更改网络。最低支持内核须随本项目程序的 hook/helper 实测确定。

因此本片不能关闭 T03，也不能作为 Linux 正式功能版本的发布证明。
