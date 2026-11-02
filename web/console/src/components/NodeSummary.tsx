import { useEffect, useState } from "react";
import { Globe2, Clock3, Activity, Shield } from "lucide-react";
import { getJson } from "../platform";
import type {
  NodesOverview,
  NodeSummary as Summary,
  ProviderNodeSummary,
} from "../nodes";
import { Stat } from "./ui";

export function NodeStats({ value }: { value: Summary }) {
  return (
    <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <Stat
        title="节点库存"
        value={String(value.inventory)}
        detail={`当前池 ${value.published} · 人工限制 ${value.restricted}`}
        icon={Globe2}
      />
      <Stat
        title="可准入节点"
        value={String(value.eligible)}
        detail="节点自身准入；入口与分组仍有过滤"
        icon={Shield}
      />
      <Stat
        title="近到期 / 已到期"
        value={`${value.near_expiry} / ${value.expired}`}
        detail="近到期窗口为 60 秒，与健康分别判断"
        icon={Clock3}
      />
      <Stat
        title="关联拨号与连接"
        value={String(value.active_connections)}
        detail={`真实在途与套接字 · 来源不可用 ${value.source_unavailable}`}
        icon={Activity}
      />
    </div>
  );
}
export function SourceStats({ sources }: { sources: ProviderNodeSummary[] }) {
  return (
    <div className="grid gap-3 lg:grid-cols-2" aria-label="来源水位与异常">
      {sources.map((source) => (
        <div key={source.id} className="panel min-w-0 space-y-2 p-4 text-xs">
          <a
            className="break-all text-sm font-medium text-blue-600"
            href={`./provider-detail.html?id=${encodeURIComponent(source.id)}`}
          >
            {source.id} →
          </a>
          <p>
            {source.enabled ? "来源已启用" : "来源已停用"} · 库存{" "}
            {source.inventory} · 可准入 {source.eligible} · 近到期{" "}
            {source.near_expiry} · 已到期 {source.expired}
          </p>
          <p className="muted">
            {source.refill_enabled
              ? `补池已配置：最小 ${source.minimum} / 目标 ${source.target} / 上限 ${source.maximum}`
              : "自动补池未启用"}
          </p>
          <p className="muted break-words">
            最近提取记录：{source.latest_task_state ?? "无"}
            {source.latest_task_id !== null ? ` #${source.latest_task_id}` : ""}
            {source.latest_task_at !== null
              ? ` · ${new Date(source.latest_task_at * 1000).toLocaleString("zh-CN")}`
              : ""}
          </p>
          {source.latest_error_code && (
            <p className="break-all text-amber-700">
              最近异常：{source.latest_error_code}
            </p>
          )}
        </div>
      ))}
    </div>
  );
}
export default function NodeSummary({ endpoint }: { endpoint: string }) {
  const [value, setValue] = useState<NodesOverview | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    const controller = new AbortController();
    void getJson<NodesOverview>("api/v1/nodes?limit=1", controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) setValue(result);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted)
          setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => controller.abort();
  }, [endpoint]);
  return (
    <section className="mt-6 space-y-3" aria-label="节点与来源摘要">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">节点与来源</h2>
        <a className="text-sm text-blue-600" href="./nodes.html">
          查看节点 →
        </a>
      </div>
      {value && (
        <>
          <NodeStats value={value.summary} />
          <SourceStats sources={value.sources} />
          <p className="muted text-xs">
            节点事实时间{" "}
            {new Date(value.evaluated_at_unix * 1000).toLocaleString("zh-CN")} ·{" "}
            {value.inventory_truncated
              ? "库存已达查询上限，摘要为受限样本"
              : "库存包含保留的退休记录"}
          </p>
        </>
      )}
      {error && (
        <p role="alert" className="text-sm text-amber-700">
          节点摘要暂不可用：{error}
        </p>
      )}
    </section>
  );
}
