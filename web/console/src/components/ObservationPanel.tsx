import { useEffect, useState } from "react";
import { getJson } from "../platform";
import type { ObservationOverview } from "../observability";
import { usePageQuery } from "../use-page-query";
import { formatBytes } from "../connections";
const date = (value: number) => new Date(value).toLocaleString("zh-CN");
export default function ObservationPanel({
  mode = "traffic",
}: {
  mode?: "traffic" | "connections" | "rules";
}) {
  const { params, navigate } = usePageQuery();
  const value = Number(params.get("observation_window"));
  const windowSecs = [300, 900, 3600].includes(value) ? value : 3600;
  const [data, setData] = useState<ObservationOverview | null>(null),
    [error, setError] = useState(""),
    [reload, setReload] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let running = false;
    setData(null);
    setError("");
    async function poll() {
      if (running || controller.signal.aborted) return;
      if (document.visibilityState !== "visible") {
        timer = setTimeout(() => void poll(), 5000);
        return;
      }
      running = true;
      try {
        const result = await getJson<ObservationOverview>(
          `api/v1/observability?window_secs=${windowSecs}`,
          controller.signal,
        );
        if (!controller.signal.aborted) {
          setData(result);
          setError("");
        }
      } catch (cause) {
        if (!controller.signal.aborted)
          setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        running = false;
        if (!controller.signal.aborted)
          timer = setTimeout(() => void poll(), 5000);
      }
    }
    const visible = () => {
      clearTimeout(timer);
      void poll();
    };
    document.addEventListener("visibilitychange", visible);
    void poll();
    return () => {
      controller.abort();
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [windowSecs, reload]);
  const name =
    mode === "traffic"
      ? "流量与失败趋势"
      : mode === "connections"
        ? "连接历史关联"
        : "业务规则命中统计";
  let elapsed = 0;
  const series =
    data?.samples.map((sample, index) => ({
      elapsed: (elapsed += index === 0 ? 0 : sample.interval_ms),
      up: (sample.bytes_up_delta * 1000) / Math.max(1, sample.interval_ms),
      down: (sample.bytes_down_delta * 1000) / Math.max(1, sample.interval_ms),
      failed: sample.failed_delta + sample.rejected_delta,
      active: sample.active,
    })) ?? [];
  return (
    <section className="panel min-w-0 space-y-4 p-4 sm:p-5" aria-label={name}>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-semibold">{name}</h2>
        <div className="flex flex-wrap gap-2">
          {mode !== "rules" && (
            <select
              aria-label="观测时间窗口"
              className="field w-auto"
              value={windowSecs}
              onChange={(event) => {
                const next = new URLSearchParams(window.location.search);
                next.set("observation_window", event.target.value);
                navigate(next);
              }}
            >
              <option value={300}>最近 5 分钟</option>
              <option value={900}>最近 15 分钟</option>
              <option value={3600}>最近 1 小时</option>
            </select>
          )}
          <button className="btn" onClick={() => setReload((n) => n + 1)}>
            刷新观测
          </button>
        </div>
      </div>
      {mode !== "traffic" && (
        <p className="muted text-xs leading-6">
          当前 Daemon 运行期的真实计数和元数据，重启后重置。采样每 5 秒、最多
          721 点 / 1 小时；页隐藏时暂停读取，不保存业务正文。
        </p>
      )}
      {error && (
        <p role="alert" className="break-all text-sm text-red-600">
          {error} · 已有数据可能过期
        </p>
      )}
      {!data && !error && <p className="muted text-sm">正在读取观测窗口…</p>}
      {data && (
        <>
          <p className="muted text-xs leading-5">
            {mode !== "traffic" && `观察实例 ${data.epoch.slice(0, 8)} · `}
            实际采样{" "}
            {data.oldest_sample_unix_ms === null
              ? "尚无"
              : date(data.oldest_sample_unix_ms)}{" "}
            — {date(data.observed_at_unix_ms)} · {data.samples.length} 点。
            {data.wall_clock_changed
              ? "检测到墙钟变化，保留期与速率仍按单调时钟计算。"
              : ""}
          </p>
          {mode === "traffic" && (
            <>
              <div className="grid gap-2.5 sm:grid-cols-2">
                {[
                  { key: "down" as const, label: "下载速度 B/s" },
                  { key: "up" as const, label: "上传速度 B/s" },
                  { key: "failed" as const, label: "每采样区间失败 / 拒绝数" },
                  { key: "active" as const, label: "采样时活跃连接数" },
                ].map(({ key, label }) => {
                  const max = Math.max(1, ...series.map((p) => p[key]));
                  const lastElapsed = series[series.length - 1]?.elapsed ?? 0;
                  const duration = Math.max(1, lastElapsed);
                  const points = series
                    .map((p) => {
                      const x = 5 + (p.elapsed / duration) * 310;
                      const y = 75 - (p[key] / max) * 70;
                      return `${x.toFixed(2)},${y.toFixed(2)}`;
                    })
                    .join(" ");
                  const lastX = 5 + (lastElapsed / duration) * 310;
                  const rate = key === "up" || key === "down";
                  return (
                    <figure className="chart-card" data-series={key} key={key}>
                      <figcaption className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1 text-xs">
                        <span className="inline-flex items-center gap-2 font-medium">
                          <span
                            className="chart-label-dot"
                            aria-hidden="true"
                          />
                          {label}
                        </span>
                        <span className="muted tabular-nums">
                          上界 {rate ? `${formatBytes(max)}/s` : Math.ceil(max)}
                        </span>
                      </figcaption>
                      <svg
                        viewBox="0 0 320 80"
                        preserveAspectRatio="none"
                        className="chart-line mt-2 h-16 w-full"
                        role="img"
                        aria-label={label}
                      >
                        <path
                          className="chart-gridlines"
                          d="M5 5H315 M5 40H315 M5 75H315"
                          aria-hidden="true"
                        />
                        {rate && series.length > 1 && (
                          <polygon
                            className="chart-area"
                            points={`5,75 ${points} ${lastX.toFixed(2)},75`}
                            aria-hidden="true"
                          />
                        )}
                        {series.length > 1 && (
                          <polyline
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="1.8"
                            strokeLinejoin="round"
                            vectorEffect="non-scaling-stroke"
                            points={points}
                          />
                        )}
                        {series.length === 1 && (
                          <circle
                            cx="5"
                            cy={75 - (series[0][key] / max) * 70}
                            r="3"
                            fill="currentColor"
                          />
                        )}
                      </svg>
                    </figure>
                  );
                })}
              </div>
              {series.length === 0 && (
                <p className="muted text-xs">此窗口尚无采样数据。</p>
              )}
              <details className="muted text-xs">
                <summary className="cursor-pointer">
                  观测说明与运行期累计
                </summary>
                <p className="mt-2 leading-5">
                  观察实例 {data.epoch.slice(0, 8)} · 本运行期上传{" "}
                  {formatBytes(data.statistics.bytes_up)} · 下载{" "}
                  {formatBytes(data.statistics.bytes_down)} · 活跃{" "}
                  {data.statistics.active} · 关闭 {data.statistics.closed} ·
                  失败 {data.statistics.failed}。
                </p>
                <p className="mt-1 leading-5">
                  每 5 秒采样，每小时最多 721
                  点，页隐藏时暂停读取，重启后计数重置。速率由相邻累计字节和单调采样间隔计算，首个点为基线。延迟曲线可在健康检查的探测历史中查看。
                </p>
              </details>
            </>
          )}
          {mode === "connections" && (
            <>
              <p className="muted text-xs leading-6">
                已结束连接按源地址 / 入口 / 目标聚合，精度{" "}
                {data.correlations.resolution_secs} 秒，保留{" "}
                {data.correlations.retention_secs}{" "}
                秒，边界可包含部分分钟。每分钟最多{" "}
                {data.correlations.group_limit_per_bucket}{" "}
                组；本窗口未能表示的关闭连接{" "}
                {data.correlations.unrepresented_closed_flows} 条。共有{" "}
                {data.correlations.total_groups} 组，最多显示 200 组
                {data.correlations.groups_truncated ? "，结果已截断" : ""}。
              </p>
              <div className="overflow-x-auto">
                <table className="w-full min-w-[650px] text-left text-xs">
                  <thead>
                    <tr>
                      <th>来源 / 入口</th>
                      <th>目标</th>
                      <th>关闭 / 失败</th>
                      <th>上传 / 下载</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.correlations.groups.map((row, index) => (
                      <tr
                        key={index}
                        className="border-t border-slate-100 dark:border-slate-800"
                      >
                        <td className="p-2">
                          {row.source_ip ?? "未知"}
                          <br />
                          {row.inbound ?? "未知"}
                        </td>
                        <td className="max-w-64 break-all">
                          {row.destination_host}:{row.destination_port}
                        </td>
                        <td>
                          {row.closed} / {row.failed}
                        </td>
                        <td>
                          {formatBytes(row.bytes_up)} /{" "}
                          {formatBytes(row.bytes_down)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {data.correlations.groups.length === 0 && (
                <p className="muted text-sm">
                  当前窗口没有已结束连接的关联记录。
                </p>
              )}
            </>
          )}
          {mode === "rules" && (
            <>
              <p className="muted text-xs leading-6">
                精确业务准入命中（包含拒绝），从{" "}
                {date(data.rule_hits.since_unix_ms)}{" "}
                累计。模拟和探测不增加命中；按配置版本分别记录。容量{" "}
                {data.rule_hits.counter_limit} 个计数器；超限未表示命中{" "}
                {data.rule_hits.untracked_hits} 次。下表按次数显示前 200 项 / 共{" "}
                {data.rule_hits.counters.length} 项。
              </p>
              <div className="overflow-x-auto">
                <table className="w-full text-left text-xs">
                  <thead>
                    <tr>
                      <th>版本</th>
                      <th>命中规则</th>
                      <th>次数</th>
                    </tr>
                  </thead>
                  <tbody>
                    {[...data.rule_hits.counters]
                      .sort((a, b) => b.hits - a.hits)
                      .slice(0, 200)
                      .map((row) => (
                        <tr
                          key={`${row.configuration_revision}:${row.rule_id}`}
                          className="border-t border-slate-100 dark:border-slate-800"
                        >
                          <td className="p-2">r{row.configuration_revision}</td>
                          <td className="break-all">
                            {row.rule_id ?? "默认动作"}
                          </td>
                          <td>{row.hits}</td>
                        </tr>
                      ))}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </>
      )}
    </section>
  );
}
