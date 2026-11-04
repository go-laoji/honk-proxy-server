import { useEffect, useRef, useState } from "react";
import {
  Activity,
  ArrowDown,
  ArrowRight,
  ArrowUp,
  CircleAlert,
} from "lucide-react";
import { getJson } from "../platform";
import {
  formatBytes,
  sampleRates,
  type ConnectionStatistics,
  type ConnectionSummary as Summary,
  type Rates,
} from "../connections";
import { Stat } from "./ui";

export function ConnectionStats({
  value,
  rates,
  compact = false,
}: {
  value: ConnectionStatistics;
  rates: Rates | null;
  compact?: boolean;
}) {
  const metrics = [
    {
      title: "下载速率",
      value: rates ? `${formatBytes(rates.down)}/s` : "采样中",
      detail: `本周期累计 ${formatBytes(value.bytes_down)}`,
      icon: ArrowDown,
      tone: "down",
    },
    {
      title: "上传速率",
      value: rates ? `${formatBytes(rates.up)}/s` : "采样中",
      detail: `本周期累计 ${formatBytes(value.bytes_up)}`,
      icon: ArrowUp,
      tone: "up",
    },
    {
      title: "活跃业务连接",
      value: String(value.active),
      detail: `完整观测 ${value.active_tracked} · 未完整观测 ${value.active_untracked}`,
      icon: Activity,
      tone: "active",
    },
    {
      title: "失败 / 规则拒绝 / 主动断开",
      value: `${value.failed} / ${value.rejected} / ${value.disconnected}`,
      detail: `本周期已开始 ${value.started} · 已打开 ${value.opened}`,
      icon: CircleAlert,
      tone: "failed",
    },
  ] as const;
  if (compact) {
    return (
      <dl
        className="panel grid grid-cols-2 gap-x-4 gap-y-2 px-3 py-2 text-xs sm:grid-cols-4"
        aria-label="连接统计"
      >
        {metrics.map(({ title, value, detail }) => (
          <div
            key={title}
            className="flex flex-wrap items-baseline gap-x-2"
            title={detail}
          >
            <dt className="muted">{title}</dt>
            <dd className="font-semibold tabular-nums">{value}</dd>
          </div>
        ))}
      </dl>
    );
  }
  return (
    <div className="metric-grid">
      {metrics.map((metric) => (
        <Stat key={metric.title} {...metric} />
      ))}
    </div>
  );
}
const attentionLabels: Record<string, string> = {
  no_running_inbound: "尚未运行代理入口",
  no_forwarding_path: "当前入口的规则或节点状态没有可转发路径",
  pending_restart: "配置包含等待重启的变更",
  dns_capture_preparation_failed: "DNS 内核接管准备失败",
  dns_capture_not_attached: "DNS 内核接管当前未挂载",
  configuration_preparation_failed: "配置运行准备失败",
  listen_preparation_failed: "代理入口监听准备失败",
  dns_listen_preparation_failed: "DNS 监听准备失败",
  routing_node_unavailable: "路由引用的节点不可用",
};
const safeHref = (value: string) =>
  /^\.\/(inbounds|rules|config|dns|provider-detail)\.html(?:\?[A-Za-z0-9_%=&.-]+)?$/.test(
    value,
  )
    ? value
    : undefined;
export default function ConnectionSummary({ endpoint }: { endpoint: string }) {
  const [value, setValue] = useState<Summary | null>(null);
  const [rates, setRates] = useState<Rates | null>(null);
  const [error, setError] = useState("");
  const previous = useRef<ConnectionStatistics | null>(null);
  useEffect(() => {
    let mounted = true;
    let running = false;
    let controller: AbortController | null = null;
    async function refresh() {
      if (running || document.hidden) return;
      running = true;
      controller = new AbortController();
      const request = controller;
      try {
        const result = await getJson<Summary>(
          "api/v1/connections/summary",
          request.signal,
        );
        if (mounted && !request.signal.aborted) {
          setRates(sampleRates(previous.current, result.statistics));
          previous.current = result.statistics;
          setValue(result);
          setError("");
        }
      } catch (cause) {
        if (mounted && !request.signal.aborted) {
          previous.current = null;
          setRates(null);
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      } finally {
        running = false;
      }
    }
    void refresh();
    const timer = window.setInterval(() => void refresh(), 2000);
    const visible = () => {
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", visible);
    return () => {
      mounted = false;
      controller?.abort();
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", visible);
      previous.current = null;
    };
  }, [endpoint]);
  return (
    <section className="space-y-3" aria-label="连接流量与异常摘要">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="font-semibold">实时流量</h2>
        <a className="text-link" href="./connections.html">
          查看连接 <ArrowRight size={14} aria-hidden="true" />
        </a>
      </div>
      {!value && !error && <p className="muted text-sm">正在读取连接摘要…</p>}
      {value && (
        <>
          <ConnectionStats value={value.statistics} rates={rates} />
          <details className="muted text-xs">
            <summary className="cursor-pointer">采样口径与时间</summary>
            <p className="mt-2 leading-6">
              入口 TCP 套接字 {value.proxy.active_connections}，与 HTTP 业务交换
              / 隧道分别统计。出口字节不含代理协商，HTTP 含转发首部，HTTPS
              目标内容保持不透明。每 2 秒采样；速率取同一观测周期内的相邻样本。
              观测周期始于{" "}
              {new Date(value.statistics.started_at_unix_ms).toLocaleString(
                "zh-CN",
              )}
              ， 最近成功时间{" "}
              {new Date(value.evaluated_at_unix_ms).toLocaleString("zh-CN")}。
            </p>
          </details>
          {value.attention.length > 0 && (
            <div
              className="panel panel-body space-y-3"
              aria-label="异常与运行事项"
            >
              <h3 className="text-sm font-semibold">运行事项</h3>
              {value.attention.map((item, index) => (
                <div
                  key={`${item.kind}:${item.resource_id}:${index}`}
                  className="flex flex-wrap items-center justify-between gap-2 text-sm"
                >
                  <span className="break-words">
                    {attentionLabels[item.code] ??
                      `${item.kind === "provider" ? "最近提取异常" : "运行记录"}：${item.code}`}
                    {item.resource_id ? ` · ${item.resource_id}` : ""}
                  </span>
                  <a
                    className="text-console-accent-ink"
                    href={safeHref(item.href)}
                  >
                    查看详情 →
                  </a>
                </div>
              ))}
            </div>
          )}
        </>
      )}
      {error && (
        <p
          role="alert"
          className="panel panel-body break-words text-sm text-console-warning"
        >
          连接摘要暂不可用：{error}。{value ? "保留上次成功快照。" : ""}
        </p>
      )}
    </section>
  );
}
