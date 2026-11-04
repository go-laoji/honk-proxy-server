import { useEffect, useState } from "react";
import { getJson } from "../platform";
import type { HealthHistory } from "../types";
import { usePageQuery } from "../use-page-query";

const date = (value: number) => new Date(value * 1000).toLocaleString("zh-CN");
export default function HealthHistoryPanel({ node }: { node?: string }) {
  const { params, navigate } = usePageQuery();
  const selectedWindow = Number(params.get("history_window"));
  const windowSecs = [300, 3600, 86400].includes(selectedWindow)
    ? selectedWindow
    : 3600;
  const before = params.get("history_before") ?? "";
  const [data, setData] = useState<HealthHistory | null>(null);
  const [error, setError] = useState("");
  const [reload, setReload] = useState(0);
  function move(next: string, seconds = windowSecs) {
    const query = new URLSearchParams(window.location.search);
    query.set("history_window", String(seconds));
    if (next) query.set("history_before", next);
    else query.delete("history_before");
    navigate(query);
  }
  useEffect(() => {
    const controller = new AbortController();
    setData(null);
    setError("");
    const query = new URLSearchParams({ limit: "100" });
    if (node) query.set("node", node);
    if (/^[1-9]\d*$/.test(before)) query.set("before", before);
    void (async () => {
      const clock = await getJson<HealthHistory>(
        "api/v1/health/history?limit=1",
        controller.signal,
      );
      query.set("since_unix", String(clock.observed_at_unix - windowSecs));
      const value = await getJson<HealthHistory>(
        `api/v1/health/history?${query}`,
        controller.signal,
      );
      if (!controller.signal.aborted) setData(value);
    })().catch((cause) => {
      if (!controller.signal.aborted)
        setError(cause instanceof Error ? cause.message : String(cause));
    });
    return () => controller.abort();
  }, [node, windowSecs, before, reload]);
  const curves = new Map<string, { time: number; value: number }[]>();
  for (const run of data?.records ?? []) {
    const result = run.result;
    if (
      !result ||
      result.checked_at === 0 ||
      [
        "unknown",
        "skipped",
        "expired",
        "unsupported",
        "queued",
        "probing",
      ].includes(result.verdict)
    )
      continue;
    const key = `${run.node_id} · ${result.transport}/${result.family} · ${result.profile_fingerprint.slice(0, 8)}`;
    const points = curves.get(key) ?? [];
    points.push({ time: result.checked_at, value: result.latency_ms });
    curves.set(key, points);
  }
  return (
    <section
      className="panel panel-body space-y-4"
      aria-label="健康历史与出口画像"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-semibold">
          健康历史与出口画像{node ? ` · ${node}` : ""}
        </h2>
        <div className="flex flex-wrap gap-2">
          <select
            aria-label="健康历史窗口"
            className="field w-auto"
            value={windowSecs}
            onChange={(event) => move("", Number(event.target.value))}
          >
            <option value={300}>最近 5 分钟</option>
            <option value={3600}>最近 1 小时</option>
            <option value={86400}>最近 24 小时</option>
          </select>
          <button className="btn" onClick={() => setReload((n) => n + 1)}>
            刷新历史
          </button>
        </div>
      </div>
      <p className="muted text-xs leading-6">
        来源为服务端持久探测记录，不从节点名推断出口。仅显式配置的纯 IP
        探测响应提供出口地址，地域未知；每页最多 100 条。未配置画像时显示未知。
      </p>
      {error && (
        <p role="alert" className="break-all text-sm text-console-danger">
          {error}
        </p>
      )}
      {!data && !error && <p className="muted text-sm">正在读取持久历史…</p>}
      {data && (
        <>
          <p className="muted text-xs leading-6">
            窗口 {date(data.since_unix)} — {date(data.observed_at_unix)} ·
            探测预算 {data.used_probes} / {data.daily_budget}（UTC 日起点{" "}
            {date(data.day_started_at)}）；最多保留 {data.retained_task_limit}{" "}
            个任务 / 目标 {data.retained_run_target}{" "}
            条运行，活跃任务另保留。存储最早记录：
            {data.oldest_retained_unix === null
              ? "无"
              : date(data.oldest_retained_unix)}
            。
          </p>
          <div className="grid gap-3 md:grid-cols-2">
            {[...curves].slice(0, 8).map(([key, points]) => {
              points.sort((a, b) => a.time - b.time);
              const first = points[0].time,
                last = points[points.length - 1].time,
                max = Math.max(1, ...points.map((p) => p.value));
              return (
                <figure
                  key={key}
                  className="min-w-0 rounded-lg bg-console-soft p-3"
                >
                  <figcaption className="break-all text-xs">
                    {key} · 探测总耗时 ms（上界 {max}）
                  </figcaption>
                  <svg
                    viewBox="0 0 300 70"
                    role="img"
                    aria-label={`${key} 探测耗时曲线`}
                    className="mt-2 h-20 w-full text-console-accent"
                  >
                    <polyline
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      points={points
                        .map(
                          (p) =>
                            `${5 + ((p.time - first) / Math.max(1, last - first)) * 290},${65 - (p.value / max) * 60}`,
                        )
                        .join(" ")}
                    />
                    {points.map((p, i) => (
                      <circle
                        key={i}
                        cx={
                          5 +
                          ((p.time - first) / Math.max(1, last - first)) * 290
                        }
                        cy={65 - (p.value / max) * 60}
                        r="2"
                        fill="currentColor"
                      >
                        <title>
                          {date(p.time)} · {p.value} ms
                        </title>
                      </circle>
                    ))}
                  </svg>
                </figure>
              );
            })}
          </div>
          {curves.size > 8 && (
            <p className="muted text-xs">
              曲线显示前 8 个节点/维度组合，请按节点查看完整结果。
            </p>
          )}
          <div className="overflow-x-auto">
            <table className="w-full min-w-[620px] text-left text-xs">
              <thead>
                <tr>
                  <th className="p-2">时间 / 节点</th>
                  <th>维度 / 状态</th>
                  <th>总耗时</th>
                  <th>出口 IP / 来源</th>
                </tr>
              </thead>
              <tbody>
                {data.records.map((run) => (
                  <tr key={run.id} className="border-t border-console-border">
                    <td className="p-2">
                      {date(run.result?.checked_at ?? run.created_at)}
                      <br />
                      {run.node_id} · #{run.id}
                    </td>
                    <td>
                      {run.result
                        ? `${run.result.transport}/${run.result.family} · ${run.result.verdict}`
                        : run.state}
                    </td>
                    <td>{run.result ? `${run.result.latency_ms} ms` : "—"}</td>
                    <td>
                      {run.result?.exit_ip ?? "未知"}
                      <br />
                      {run.result?.exit_ip
                        ? "配置的纯 IP 探测响应；地域未知"
                        : "未获得出口观察"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {data.records.length === 0 && (
            <p className="muted text-sm">当前窗口没有持久探测记录。</p>
          )}
          <div className="flex gap-2">
            <button className="btn" disabled={!before} onClick={() => move("")}>
              最新记录
            </button>
            <button
              className="btn"
              disabled={data.next_before === null}
              onClick={() => move(String(data.next_before))}
            >
              更早记录
            </button>
          </div>
        </>
      )}
    </section>
  );
}
