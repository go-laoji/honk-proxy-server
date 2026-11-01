import { useEffect, useRef, useState } from "react";
import { Activity, ArrowDownUp, Clock3, CircleAlert } from "lucide-react";
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
}: {
  value: ConnectionStatistics;
  rates: Rates | null;
}) {
  return (
    <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <Stat
        title="活动业务交换 / 隧道"
        value={String(value.active)}
        detail={`完整观测 ${value.active_tracked} · 未完整观测 ${value.active_untracked}`}
        icon={Activity}
      />
      <Stat
        title="出口上传 / 下载速率"
        value={
          rates
            ? `${formatBytes(rates.up)}/s / ${formatBytes(rates.down)}/s`
            : "采样中"
        }
        detail={
          rates
            ? `单调时间差 ${Math.round(rates.interval)} ms，非瞬时带宽上限`
            : "需要同一观测周期的两个样本"
        }
        icon={ArrowDownUp}
      />
      <Stat
        title="累计上传 / 下载"
        value={`${formatBytes(value.bytes_up)} / ${formatBytes(value.bytes_down)}`}
        detail={`本次观测周期始于 ${new Date(value.started_at_unix_ms).toLocaleString("zh-CN")}`}
        icon={Clock3}
      />
      <Stat
        title="失败 / 规则拒绝 / 主动断开"
        value={`${value.failed} / ${value.rejected} / ${value.disconnected}`}
        detail={`本周期已开始 ${value.started} · 已打开 ${value.opened}`}
        icon={CircleAlert}
      />
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
    <section className="mt-6 space-y-3" aria-label="连接流量与异常摘要">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">连接、流量与运行事项</h2>
        <a className="text-sm text-blue-600" href="./connections.html">
          查看连接 →
        </a>
      </div>
      {value && (
        <>
          <ConnectionStats value={value.statistics} rates={rates} />
          <p className="muted text-xs leading-6">
            入口 TCP 套接字 {value.proxy.active_connections}，与 HTTP 业务交换 /
            隧道分别统计。出口字节不含代理协商，HTTP 含转发首部，HTTPS
            目标内容保持不透明。每 2 秒采样；最近成功时间{" "}
            {new Date(value.evaluated_at_unix_ms).toLocaleString("zh-CN")}。
          </p>
          {value.attention.length > 0 && (
            <div className="panel space-y-3 p-4" aria-label="异常与运行事项">
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
                  <a className="text-blue-600" href={safeHref(item.href)}>
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
          className="panel break-words p-4 text-sm text-amber-700"
        >
          连接摘要暂不可用：{error}。{value ? "保留上次成功快照。" : ""}
        </p>
      )}
    </section>
  );
}
