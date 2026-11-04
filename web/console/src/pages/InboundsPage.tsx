import { useState } from "react";
import { Activity, Cable, CircleHelp, Gauge, Play, Square } from "lucide-react";
import { Badge, Details, Stat } from "../components/ui";
import { postJson } from "../platform";
import type { ServiceData } from "../use-service";
import type { InboundStatus } from "../types";

const rate = (value: number) =>
  value ? `${value.toLocaleString()} B/s` : "不限";
const date = (value: number) => new Date(value * 1000).toLocaleString("zh-CN");

export default function InboundsPage({
  data,
  refresh,
}: {
  data: ServiceData;
  refresh: () => Promise<void>;
}) {
  const [pending, setPending] = useState("");
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const { proxy } = data;
  const running = proxy.inbounds.filter((entry) => entry.running).length;
  async function toggle(entry: InboundStatus) {
    setPending(entry.id);
    setError("");
    setMessage("");
    try {
      const result = await postJson<InboundStatus>(
        `api/v1/inbounds/${encodeURIComponent(entry.id)}/${entry.running ? "stop" : "start"}`,
      );
      setMessage(
        `${entry.id} 已${result.running ? "启动" : "停止"}，运行操作不写回启动配置。`,
      );
      await refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPending("");
    }
  }
  return (
    <>
      <div className="metric-grid">
        <Stat
          title="运行入口"
          value={`${running} / ${proxy.inbounds.length}`}
          detail="以实际监听状态为准"
          icon={Cable}
        />
        <Stat
          title="活跃连接"
          value={String(proxy.active_connections)}
          detail={`全局上限 ${proxy.limits.max_connections}`}
          icon={Activity}
        />
        <Stat
          title="默认选择可用上游"
          value={String(proxy.eligible_upstreams)}
          detail="遵守组范围、健康策略与剩余寿命；直连另由规则决定"
          icon={Gauge}
        />
        <Stat
          title="全局字节速率"
          value={rate(proxy.limits.bytes_per_second)}
          detail="所有入口共享，业务流量双向合计"
          icon={Gauge}
        />
      </div>
      <div className="panel panel-body mt-3 flex items-start gap-3 text-sm leading-7">
        <CircleHelp
          size={19}
          className="mt-1 shrink-0 text-console-accent-ink"
        />
        <p className="muted">
          入口、上游与资源限制可通过配置工作台校验并激活版本。此页启停立即生效，停止会断开该入口的现有连接。
          SOCKS5 支持 TCP CONNECT 与 UDP ASSOCIATE；UDP 控制 TCP
          关闭即释放，FRAG 不支持，每个关联最多 32 个目标且受全局/入口额度约束。
          热更新在 enabled 未变化时保留手动启停；Daemon 重启按已持久化配置恢复。
        </p>
      </div>
      {!proxy.eligible_upstreams && (
        <p className="mt-5 rounded-xl border border-amber-200 bg-amber-50 p-4 text-sm leading-7 text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          默认选择当前没有合格的代理池节点。规则可以明确直连或选择其他组，其余目标按规则处理；直连需要明确配置。
        </p>
      )}
      {error && (
        <p
          role="alert"
          className="mt-5 rounded-xl bg-red-50 p-4 text-sm text-red-700 dark:bg-red-950 dark:text-red-200"
        >
          {error}。操作结果以刷新后的服务状态为准。
        </p>
      )}
      {message && (
        <p
          role="status"
          className="mt-5 text-sm text-emerald-700 dark:text-emerald-300"
        >
          {message}
        </p>
      )}
      <div className="mt-3 grid gap-3 xl:grid-cols-2">
        {proxy.inbounds.map((entry) => (
          <section
            key={entry.id}
            className="panel min-w-0"
            aria-label={`入口 ${entry.id}`}
          >
            <div className="panel-header panel-toolbar">
              <div>
                <h2 className="font-semibold">{entry.id}</h2>
                <p className="muted mt-1 text-xs">
                  {entry.protocol.toUpperCase()} · 无需代理账号
                </p>
              </div>
              <div className="flex items-center gap-3">
                <Badge active={entry.running}>
                  {entry.running ? "监听中" : "已停止"}
                </Badge>
                <button
                  className="btn"
                  disabled={Boolean(pending) || proxy.shutting_down}
                  aria-label={`${entry.running ? "停止" : "启动"} ${entry.id}`}
                  onClick={() => void toggle(entry)}
                >
                  {entry.running ? <Square size={14} /> : <Play size={14} />}
                  {pending === entry.id
                    ? "处理中…"
                    : entry.running
                      ? "停止"
                      : "启动"}
                </button>
              </div>
            </div>
            <Details
              rows={[
                ["配置监听", <code>{entry.configured_listen}</code>],
                ["实际监听", <code>{entry.listen ?? "未监听"}</code>],
                ["启动时启用", entry.configured_enabled ? "是" : "否"],
                [
                  "手动运行覆盖",
                  entry.runtime_override == null
                    ? "未设置"
                    : entry.runtime_override
                      ? "启动"
                      : "停止",
                ],
                [
                  "连接使用",
                  `${entry.active_connections} / ${entry.max_connections}`,
                ],
                ["入口字节速率", rate(entry.bytes_per_second)],
                ["累计接入", String(entry.accepted_connections)],
                [
                  "拒绝 · 全局 / 入口上限",
                  `${entry.rejected_global_limit} / ${entry.rejected_inbound_limit}`,
                ],
                [
                  "失败 / 空闲回收",
                  `${entry.failed_connections} / ${entry.idle_timeouts}`,
                ],
                ...(entry.certificate
                  ? ([
                      [
                        "TLS 证书有效期",
                        entry.certificate.valid_now
                          ? "有效期内"
                          : "尚未生效或已过期",
                      ],
                      ["证书开始", date(entry.certificate.not_before_unix)],
                      ["证书到期", date(entry.certificate.not_after_unix)],
                    ] as [string, string][])
                  : []),
                ...(entry.last_error
                  ? ([["最近错误", entry.last_error]] as [string, string][])
                  : []),
              ]}
            />
            {entry.listen && (
              <div className="border-t border-console-border p-5">
                <p className="muted mb-2 text-xs">curl 接入示例</p>
                <pre className="code-block">
                  <code>{`curl --proxy ${entry.protocol === "socks5" ? "socks5h" : entry.protocol}://${entry.listen} https://example.com/`}</code>
                </pre>
              </div>
            )}
          </section>
        ))}
      </div>
      {!proxy.inbounds.length && (
        <section className="panel panel-body mt-3">
          <h2 className="font-semibold">尚未配置代理入口</h2>
          <p className="muted mt-3 text-sm leading-7">
            在启动 TOML
            添加下面的入口配置，并配置实际供应商上游。只有入口运行且存在可准入上游时，代理才报告就绪。
          </p>
          <pre className="code-block mt-4">
            <code>
              {
                '[[proxy.inbounds]]\nid = "browser-http"\nprotocol = "http"\nlisten = "127.0.0.1:17901"\nenabled = true'
              }
            </code>
          </pre>
        </section>
      )}
      <div className="mt-3 grid gap-3 xl:grid-cols-2">
        <section className="panel min-w-0">
          <h2 className="panel-header">有效资源限制</h2>
          <Details
            rows={[
              [
                "转发缓冲 · 每方向",
                `${proxy.limits.buffer_bytes.toLocaleString()} B`,
              ],
              [
                "HTTP 头缓冲上限",
                `${proxy.limits.max_header_bytes.toLocaleString()} B`,
              ],
              [
                "握手 / 上游连接超时",
                `${proxy.limits.handshake_timeout_ms} / ${proxy.limits.connect_timeout_ms} ms`,
              ],
              ["空闲超时", `${proxy.limits.idle_timeout_ms} ms`],
              [
                "最小剩余租约",
                `${proxy.limits.min_remaining_lifetime_secs} 秒`,
              ],
            ]}
          />
          <p className="muted p-5 text-xs leading-6">
            满额拒绝新连接并记录命中计数。0
            字节速率表示不限；限速等待不算空闲。停止会取消活动转发并回收额度。
          </p>
        </section>
        <section className="panel panel-body min-w-0">
          <h2 className="font-semibold">浏览器与系统接入</h2>
          <div className="muted mt-4 space-y-4 text-sm leading-7">
            <p>
              <strong>HTTP / CONNECT：</strong>
              在浏览器或系统填写代理主机和端口，HTTPS 目标使用同一 HTTP 入口，经
              CONNECT 保留端到端 TLS。
            </p>
            <p>
              <strong>HTTPS 代理：</strong>
              客户端到代理之间先建立 TLS，需客户端支持 https:// 代理、信任签发
              CA，访问名称必须匹配证书。系统界面的“HTTPS
              代理”有时指目标类型，请区分。curl 可使用 --proxy-cacert 提供代理
              CA。
            </p>
            <p>
              <strong>SOCKS5 TCP / UDP：</strong>
              选择无认证 SOCKS5；支持域名、IPv4、IPv6 CONNECT 与 UDP
              ASSOCIATE。BIND 和 UDP 分片不支持。UDP
              必须由客户端显式建立关联；控制 TCP 关闭即释放。HTTP/HTTPS
              上游不承载 UDP，出口需 SOCKS5 或显式直连规则。浏览器自身的
              DoH、DNS 与绕过配置仍会影响流量。
            </p>
            <p>
              0.0.0.0 / [::] 是监听范围，接入时替换为可达的 Daemon
              主机地址；127.0.0.1 仅适用于 Daemon
              本机。所有入口均无需本项目账号，上游供应商凭据由服务端使用。
            </p>
          </div>
        </section>
      </div>
    </>
  );
}
