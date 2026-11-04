import { useEffect, useRef, useState, type FormEvent } from "react";
import { FileCode2, Filter, Search, X } from "lucide-react";
import { AdvancedFilters, FilterStartTime } from "../components/FilterOptions";
import { Badge, Details } from "../components/ui";
import AlertPanel from "../components/AlertPanel";
import DiagnosticPanel from "../components/DiagnosticPanel";
import FeedControls from "../components/FeedControls";
import { usePageQuery } from "../use-page-query";
import { useEventFeed } from "../use-event-feed";
import {
  dateMs,
  eventLevels,
  eventModules,
  queryString,
  resetReasons,
  type EventFrame,
  type EventQuery,
  type EventsOverview,
  type ServiceEvent,
} from "../tasks";

const keys = [
  "cursor",
  "level",
  "module",
  "task_id",
  "request_id",
  "node_id",
  "provider_id",
  "after_unix_ms",
  "limit",
];
const descriptions: Record<string, string> = {
  management_operation: "管理接口处理",
  task_queued: "任务已排队",
  task_running: "任务进度更新",
  task_completed: "任务处理完成",
  task_failed: "任务失败",
  task_outcome_unknown: "任务结果不确定",
  task_interrupted: "任务被中断",
  task_discarded: "任务已丢弃",
  task_pending_restart: "任务等待重启",
  probe_probing: "探测开始",
  probe_completed: "探测完成",
  probe_discarded: "探测被丢弃",
  probe_interrupted: "探测被中断",
  node_enabled: "节点解除限制",
  node_disabled: "节点禁用新准入",
  node_draining: "节点停止接收新业务",
  node_deleted: "节点历史停用",
  configuration_revision_saved: "配置候选已保存",
  configuration_draft_saved: "共享草稿已保存",
};

export default function EventsPage() {
  const { params, navigate, search } = usePageQuery();
  const query: EventQuery = {};
  for (const key of keys) {
    const text = params.get(key);
    if (text)
      (query as Record<string, string | number>)[key] = [
        "task_id",
        "after_unix_ms",
        "limit",
      ].includes(key)
        ? Number(text)
        : text;
  }
  const queryKey = `${queryString(query)}|${params.get("epoch") ?? ""}`;
  const [automatic, setAutomatic] = useState(true);
  const [cache, setCache] = useState<{
    key: string;
    page: EventsOverview;
    records: ServiceEvent[];
  } | null>(null);
  const [selected, setSelected] = useState<ServiceEvent | null>(null);
  const detailRegion = useRef<HTMLElement>(null);
  const [notice, setNotice] = useState("");
  const lastEpoch = useRef<string | null>(null);
  useEffect(() => setSelected(null), [queryKey]);
  useEffect(() => {
    if (selected) detailRegion.current?.focus();
  }, [selected]);
  const feed = useEventFeed(
    query,
    automatic,
    (frame: EventFrame) => {
      if (frame.kind === "unavailable") return;
      const page = frame.data;
      if (lastEpoch.current && lastEpoch.current !== page.epoch)
        setSelected(null);
      lastEpoch.current = page.epoch;
      if (page.reset_reason) {
        setNotice(resetReasons[page.reset_reason] ?? "事件已重新建立快照。");
        setSelected(null);
      }
      setCache((current) => {
        const reset =
          !current ||
          current.key !== queryKey ||
          current.page.epoch !== page.epoch ||
          frame.kind === "snapshot" ||
          frame.kind === "reset";
        const records = new Map<number, ServiceEvent>(
          (reset ? [] : current.records).map((r) => [r.sequence, r]),
        );
        for (const record of page.records) records.set(record.sequence, record);
        return {
          key: queryKey,
          page,
          records: Array.from(records.values())
            .sort((a, b) => a.sequence - b.sequence)
            .slice(-500),
        };
      });
    },
    params.get("epoch") ?? undefined,
  );
  const value = cache?.key === queryKey ? cache : null;
  const records = value?.records.slice(-100).reverse() ?? [];
  function filters(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const next = new URLSearchParams();
    for (const key of keys) {
      const v = String(data.get(key) ?? "").trim();
      if (v) next.set(key, v);
    }
    if (params.has("epoch")) next.set("epoch", params.get("epoch")!);
    setSelected(null);
    setNotice("");
    navigate(next);
    feed.reconnect();
  }
  function currentInstance() {
    const next = new URLSearchParams(search);
    next.delete("epoch");
    setSelected(null);
    setNotice("");
    navigate(next);
    feed.reconnect();
  }
  return (
    <div className="space-y-3">
      <section id="service-events" className="panel space-y-2.5 p-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h2 className="flex items-center gap-2 text-sm font-semibold">
            <FileCode2 size={19} />
            结构化服务事件
          </h2>
          <nav className="flex flex-wrap gap-3" aria-label="日志与事件页内导航">
            <a className="text-link" href="#service-alerts">
              状态告警
            </a>
            <a className="text-link" href="#service-diagnostics">
              诊断
            </a>
          </nav>
        </div>
        <FeedControls
          state={feed.state}
          automatic={automatic}
          toggle={() => setAutomatic((v) => !v)}
          reconnect={feed.reconnect}
        />
        <form
          key={search}
          aria-label="事件筛选"
          onSubmit={filters}
          className="space-y-2"
        >
          <div className="filter-grid">
            <label className="text-xs">
              级别
              <select
                className="field mt-1"
                name="level"
                defaultValue={params.get("level") ?? ""}
              >
                <option value="">全部级别</option>
                {Object.entries(eventLevels).map(([key, label]) => (
                  <option value={key} key={key}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <label className="text-xs">
              模块
              <select
                className="field mt-1"
                name="module"
                defaultValue={params.get("module") ?? ""}
              >
                <option value="">全部模块</option>
                {Object.entries(eventModules).map(([key, label]) => (
                  <option value={key} key={key}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <label className="text-xs">
              统一任务 ID
              <input
                className="field mt-1"
                name="task_id"
                type="number"
                min="1"
                defaultValue={params.get("task_id") ?? ""}
              />
            </label>
            <div className="filter-actions">
              <button className="btn">
                <Filter size={15} />
                应用事件筛选
              </button>
              <button
                type="button"
                className="btn"
                onClick={() => {
                  setSelected(null);
                  setNotice("");
                  navigate(new URLSearchParams());
                  feed.reconnect();
                }}
              >
                重置筛选
              </button>
            </div>
          </div>
          <AdvancedFilters
            active={
              params.has("request_id") ||
              params.has("node_id") ||
              params.has("provider_id") ||
              params.has("after_unix_ms") ||
              (params.get("limit") ?? "100") !== "100" ||
              params.has("cursor")
            }
          >
            <label className="text-xs">
              请求 ID
              <input
                className="field mt-1"
                name="request_id"
                defaultValue={params.get("request_id") ?? ""}
              />
            </label>
            <label className="text-xs">
              节点 ID
              <input
                className="field mt-1"
                name="node_id"
                defaultValue={params.get("node_id") ?? ""}
              />
            </label>
            <label className="text-xs">
              供应商 ID
              <input
                className="field mt-1"
                name="provider_id"
                defaultValue={params.get("provider_id") ?? ""}
              />
            </label>
            <FilterStartTime
              name="after_unix_ms"
              value={params.get("after_unix_ms") ?? ""}
            />
            <label className="text-xs">
              每批最多
              <input
                className="field mt-1"
                name="limit"
                type="number"
                min="1"
                max="200"
                defaultValue={params.get("limit") ?? "100"}
              />
            </label>
            <label className="text-xs sm:col-span-2">
              从游标恢复（可选）
              <input
                className="field mt-1"
                name="cursor"
                defaultValue={params.get("cursor") ?? ""}
              />
            </label>
          </AdvancedFilters>
        </form>
        {params.has("epoch") && (
          <div className="muted flex flex-wrap items-center gap-2 break-all text-xs">
            关联实例：{params.get("epoch")}
            <button className="btn text-xs" onClick={currentInstance}>
              清除实例限制并重新查询
            </button>
          </div>
        )}
        {value && (
          <details className="muted text-xs">
            <summary className="cursor-pointer">
              显示 {records.length} 条 · 最近读取{" "}
              {dateMs(value.page.observed_at_unix_ms)}
            </summary>
            <p className="mt-2 break-all leading-5">
              保留上限 {value.page.retention_limit} 条 · 本页缓存{" "}
              {value.records.length} 条
              {value.page.has_more ? " · 仍有待补事件" : ""}
              <br />
              观测周期 {value.page.epoch}
              <br />
              完整游标 {value.page.cursor}
            </p>
          </details>
        )}
      </section>
      {notice && (
        <p
          role="status"
          className="panel break-words p-4 text-sm text-amber-700"
        >
          {notice}
        </p>
      )}
      {selected && value && (
        <section
          ref={detailRegion}
          tabIndex={-1}
          className="panel"
          aria-label="事件详情"
        >
          <div className="flex items-center justify-between p-5">
            <h2 className="font-semibold">事件 #{selected.sequence}</h2>
            <button className="btn" onClick={() => setSelected(null)}>
              <X size={14} />
              关闭事件详情
            </button>
          </div>
          <Details
            rows={[
              ["记录时间", dateMs(selected.occurred_at_unix_ms)],
              ["记录时的观测周期", selected.recorded_epoch],
              [
                "级别 / 模块",
                `${eventLevels[selected.level] ?? selected.level} / ${eventModules[selected.module] ?? selected.module}`,
              ],
              ["类别", selected.code],
              [
                "资源",
                `${selected.resource_kind ?? "未记录"} / ${selected.resource_id ?? "未记录"}`,
              ],
              ["请求 ID", selected.request_id ?? "未记录"],
              ["TCP 对端", selected.peer ?? "未记录；可能为后台状态事件"],
              [
                "动作 / 结果",
                `${selected.action ?? "未记录"} / ${selected.outcome ?? "未记录"}`,
              ],
              [
                "配置版本",
                selected.configuration_revision === null
                  ? "未记录"
                  : `r${selected.configuration_revision}`,
              ],
              [
                "统一任务",
                selected.task_id ? (
                  <a
                    className="text-blue-600"
                    href={`./tasks.html?detail=${selected.task_id}&epoch=${value.page.epoch}`}
                  >
                    #{selected.task_id} · 版本{" "}
                    {selected.task_version ?? "未记录"} →
                  </a>
                ) : (
                  "无关联任务"
                ),
              ],
              [
                "节点",
                selected.node_id ? (
                  <a
                    className="text-blue-600"
                    href={`./node-detail.html?id=${encodeURIComponent(selected.node_id)}`}
                  >
                    {selected.node_id} →
                  </a>
                ) : (
                  "无关联节点"
                ),
              ],
              [
                "供应商",
                selected.provider_id ? (
                  <a
                    className="text-blue-600"
                    href={`./provider-detail.html?id=${encodeURIComponent(selected.provider_id)}`}
                  >
                    {selected.provider_id} →
                  </a>
                ) : (
                  "无关联来源"
                ),
              ],
            ]}
          />
        </section>
      )}
      <div
        className={records.length ? "panel" : undefined}
        aria-label="服务事件列表"
      >
        {records.map((event) => (
          <article
            key={event.sequence}
            aria-label={`事件 ${event.sequence}`}
            className="management-record"
          >
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <strong className="break-words text-sm">
                  {descriptions[event.code] ?? event.code}
                </strong>
                <Badge active={event.level === "info"}>
                  {eventLevels[event.level] ?? event.level}
                </Badge>
              </div>
              <p className="muted mt-0.5 break-words text-xs">
                #{event.sequence} · {dateMs(event.occurred_at_unix_ms)} ·{" "}
                {eventModules[event.module] ?? event.module} · {event.code}
              </p>
            </div>
            <p className="muted min-w-0 break-words text-xs [overflow-wrap:anywhere]">
              {event.resource_kind ?? "资源未记录"}
              {event.resource_id ? ` / ${event.resource_id}` : ""}
              {event.task_id
                ? ` · 任务 #${event.task_id} v${event.task_version ?? "?"}`
                : ""}
              {event.request_id ? ` · 请求 ${event.request_id}` : ""}
            </p>
            <div className="flex flex-wrap items-center gap-2">
              <button
                className="btn text-xs"
                onClick={() => setSelected(event)}
              >
                <Search size={13} />
                事件详情
              </button>
              {event.task_id && value && (
                <a
                  className="text-link"
                  href={`./tasks.html?detail=${event.task_id}&epoch=${value.page.epoch}`}
                >
                  查看统一任务 →
                </a>
              )}
            </div>
          </article>
        ))}
      </div>
      {value && !records.length && (
        <p className="panel p-8 text-center text-sm">
          当前保留窗口内没有符合筛选的事件。后台任务仍由服务独立运行。
        </p>
      )}
      <div id="service-alerts">
        <AlertPanel />
      </div>
      <div id="service-diagnostics">
        <DiagnosticPanel />
      </div>
    </div>
  );
}
