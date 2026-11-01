import { useEffect, useRef, useState, type FormEvent } from "react";
import { Filter, ListChecks, RefreshCw, Search, X } from "lucide-react";
import { ApiRequestError, getJson } from "../platform";
import { Badge, Details } from "../components/ui";
import FeedControls from "../components/FeedControls";
import TaskSubmission from "../components/TaskSubmission";
import { usePageQuery } from "../use-page-query";
import { useEventFeed } from "../use-event-feed";
import {
  dateMs,
  queryString,
  resetReasons,
  sourceLink,
  taskErrors,
  taskKinds,
  taskStates,
  type TaskInfo,
  type TasksOverview,
} from "../tasks";

const keys = [
  "source_ref",
  "kind",
  "state",
  "resource_kind",
  "resource_id",
  "request_id",
  "created_after_unix_ms",
  "before",
  "limit",
];
export default function TasksPage() {
  const { params, navigate, search } = usePageQuery();
  const [epoch, setEpoch] = useState<string | null>(null),
    [automatic, setAutomatic] = useState(true),
    [refresh, setRefresh] = useState(0);
  const [result, setResult] = useState<{
      key: string;
      value: TasksOverview;
    } | null>(null),
    [loading, setLoading] = useState(true),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [detail, setDetail] = useState<TaskInfo | null>(null),
    [detailError, setDetailError] = useState("");
  const lastEpoch = useRef<string | null>(null);
  const invalidation = useRef<ReturnType<typeof setTimeout> | null>(null);
  const query: Record<string, string | number | undefined> = {};
  for (const key of keys) {
    const value = params.get(key);
    if (value) query[key] = value;
  }
  const expected = params.get("epoch") ?? epoch ?? undefined;
  query.expected_epoch = expected;
  const apiQuery = queryString(query);
  const detailId = params.get("detail");
  const feed = useEventFeed(
    { limit: 200 },
    automatic,
    (frame) => {
      if (frame.kind === "unavailable") return;
      const page = frame.data;
      if (lastEpoch.current && lastEpoch.current !== page.epoch) {
        setResult(null);
        setDetail(null);
        setNotice(resetReasons.instance_changed);
        const next = new URLSearchParams(window.location.search);
        next.delete("detail");
        next.delete("before");
        navigate(next, true);
      }
      lastEpoch.current = page.epoch;
      setEpoch(page.epoch);
      if (page.reset_reason)
        setNotice(
          resetReasons[page.reset_reason] ??
            "事件快照已恢复，任务列表将重新读取。",
        );
      if (
        page.records.some((event) => event.task_id !== null) ||
        frame.kind === "snapshot" ||
        frame.kind === "reset"
      ) {
        if (invalidation.current === null)
          invalidation.current = setTimeout(() => {
            invalidation.current = null;
            setRefresh((n) => n + 1);
          }, 100);
      }
    },
    params.get("epoch") ?? undefined,
  );
  useEffect(
    () => () => {
      if (invalidation.current !== null) clearTimeout(invalidation.current);
      invalidation.current = null;
    },
    [],
  );
  useEffect(() => {
    if (!epoch) return;
    const controller = new AbortController();
    setLoading(true);
    setError("");
    void getJson<TasksOverview>(`api/v1/tasks?${apiQuery}`, controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) setResult({ key: apiQuery, value });
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return;
        setError(cause instanceof Error ? cause.message : String(cause));
        if (
          cause instanceof ApiRequestError &&
          cause.code === "task_instance_changed" &&
          !params.has("epoch")
        )
          feed.reconnect();
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [apiQuery, epoch, refresh]);
  useEffect(() => {
    setDetail(null);
    setDetailError("");
    if (!detailId || !epoch) return;
    const controller = new AbortController();
    void getJson<TaskInfo>(
      `api/v1/tasks/${encodeURIComponent(detailId)}?expected_epoch=${encodeURIComponent(expected ?? epoch)}`,
      controller.signal,
    )
      .then((value) => {
        if (!controller.signal.aborted) setDetail(value);
      })
      .catch((cause) => {
        if (!controller.signal.aborted)
          setDetailError(
            cause instanceof Error ? cause.message : String(cause),
          );
      });
    return () => controller.abort();
  }, [detailId, epoch, expected, refresh]);
  const value = result?.key === apiQuery ? result.value : null;
  function filters(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const next = new URLSearchParams();
    for (const key of keys) {
      if (key === "before") continue;
      const v = String(data.get(key) ?? "").trim();
      if (v) next.set(key, v);
    }
    if (params.has("epoch")) next.set("epoch", params.get("epoch")!);
    navigate(next);
    setRefresh((n) => n + 1);
  }
  function show(id: number | null, context = epoch) {
    const next = new URLSearchParams(window.location.search);
    if (id) {
      next.set("detail", String(id));
      if (context) next.set("epoch", context);
    } else next.delete("detail");
    navigate(next);
  }
  function submitted(requestId: string, context: string) {
    const next = new URLSearchParams();
    next.set("request_id", requestId);
    next.set("epoch", context);
    navigate(next);
    setRefresh((n) => n + 1);
  }
  function accepted(task: TaskInfo, context: string) {
    show(task.id, context);
    setRefresh((n) => n + 1);
  }
  function unbind() {
    const next = new URLSearchParams(search);
    next.delete("epoch");
    next.delete("detail");
    next.delete("before");
    setDetail(null);
    navigate(next);
    feed.reconnect();
    setRefresh((n) => n + 1);
  }
  const source = detail && epoch ? sourceLink(detail, epoch) : null;
  return (
    <div className="space-y-6">
      <section className="panel space-y-4 p-5">
        <div>
          <h2 className="flex items-center gap-2 font-semibold">
            <ListChecks size={19} />
            统一后台任务
          </h2>
          <p className="muted mt-2 text-xs leading-6">
            供应商、健康检测、激活和诊断共用持久任务视图。离开页面只停止等待，已接受的工作由服务持有。完成检测不等于全部节点健康；提取结果不确定时不会自动重放购买。
          </p>
        </div>
        <FeedControls
          state={feed.state}
          automatic={automatic}
          toggle={() => setAutomatic((v) => !v)}
          reconnect={() => {
            feed.reconnect();
            setRefresh((n) => n + 1);
          }}
        />
        <form
          key={search}
          onSubmit={filters}
          className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4"
          aria-label="任务筛选"
        >
          <label className="text-xs">
            任务类型
            <select
              className="field mt-1"
              name="kind"
              defaultValue={params.get("kind") ?? ""}
            >
              <option value="">全部类型</option>
              {Object.entries(taskKinds).map(([key, label]) => (
                <option key={key} value={key}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label className="text-xs">
            任务状态
            <select
              className="field mt-1"
              name="state"
              defaultValue={params.get("state") ?? ""}
            >
              <option value="">全部状态</option>
              {Object.entries(taskStates).map(([key, label]) => (
                <option key={key} value={key}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label className="text-xs">
            来源记录引用
            <input
              className="field mt-1"
              name="source_ref"
              placeholder="provider:1 / health:1 / activation:1"
              defaultValue={params.get("source_ref") ?? ""}
            />
          </label>
          <label className="text-xs">
            请求 / 幂等键
            <input
              className="field mt-1"
              name="request_id"
              defaultValue={params.get("request_id") ?? ""}
            />
          </label>
          <label className="text-xs">
            资源类型
            <input
              className="field mt-1"
              name="resource_kind"
              defaultValue={params.get("resource_kind") ?? ""}
            />
          </label>
          <label className="text-xs">
            资源 ID
            <input
              className="field mt-1"
              name="resource_id"
              defaultValue={params.get("resource_id") ?? ""}
            />
          </label>
          <label className="text-xs">
            开始时间不早于（Unix ms）
            <input
              className="field mt-1"
              name="created_after_unix_ms"
              type="number"
              min="0"
              defaultValue={params.get("created_after_unix_ms") ?? ""}
            />
          </label>
          <label className="text-xs">
            每页
            <input
              className="field mt-1"
              name="limit"
              type="number"
              min="1"
              max="200"
              defaultValue={params.get("limit") ?? "50"}
            />
          </label>
          <div className="flex flex-wrap items-end gap-2">
            <button className="btn">
              <Filter size={15} />
              应用任务筛选
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => {
                navigate(new URLSearchParams());
                setRefresh((n) => n + 1);
              }}
            >
              重置任务筛选
            </button>
          </div>
        </form>
        {params.has("epoch") && (
          <div className="muted flex flex-wrap items-center gap-2 break-all text-xs">
            关联实例：{params.get("epoch")}
            <button className="btn text-xs" onClick={unbind}>
              清除实例限制并重新查询
            </button>
          </div>
        )}
        {value && (
          <p className="muted break-all text-xs leading-6">
            查询时刻：{dateMs(value.observed_at_unix_ms)} · 观测周期{" "}
            {value.epoch}。按登记 ID 降序；终态最多保留{" "}
            {value.terminal_retention_limit}{" "}
            条，进行中的源任务另受各自容量约束。列表不包含完整诊断结果，详情按需读取。
          </p>
        )}
      </section>
      <TaskSubmission onRequest={submitted} onAccepted={accepted} />
      {notice && (
        <p
          role="status"
          className="panel break-words p-4 text-sm text-amber-700"
        >
          {notice}
        </p>
      )}
      {error && (
        <p role="alert" className="panel break-words p-4 text-sm text-red-600">
          任务读取失败：{error}
        </p>
      )}
      {detailId && (
        <section className="panel" aria-label="统一任务详情">
          <div className="flex items-center justify-between gap-3 p-5">
            <h2 className="font-semibold">任务 #{detailId}</h2>
            <button className="btn" onClick={() => show(null)}>
              <X size={14} />
              关闭任务详情
            </button>
          </div>
          {detailError ? (
            <p className="break-words p-5 text-sm text-amber-700">
              {detailError}
            </p>
          ) : detail && String(detail.id) === detailId ? (
            <>
              <Details
                rows={[
                  [
                    "类型 / 终态",
                    `${taskKinds[detail.kind] ?? detail.kind} / ${detail.terminal ? "已终结" : "尚未终结"}`,
                  ],
                  [
                    "状态 / 阶段",
                    `${taskStates[detail.state] ?? detail.state} / ${detail.phase}`,
                  ],
                  ["单调版本", detail.version],
                  ["来源记录", detail.source_ref ?? "服务持有的独立工作"],
                  ["触发", detail.trigger],
                  ["请求 / 幂等键", detail.request_id ?? "未记录"],
                  [
                    "资源",
                    `${detail.resource_kind ?? "未记录"} / ${detail.resource_id ?? "未记录"}`,
                  ],
                  [
                    "配置版本",
                    detail.configuration_revision
                      ? `r${detail.configuration_revision}`
                      : "未记录；不使用当前版本补写历史",
                  ],
                  ["配置指纹", detail.configuration_fingerprint ?? "未记录"],
                  [
                    "创建 / 开始",
                    `${dateMs(detail.created_at_unix_ms)} / ${dateMs(detail.started_at_unix_ms)}`,
                  ],
                  [
                    "更新 / 结束",
                    `${dateMs(detail.updated_at_unix_ms)} / ${dateMs(detail.finished_at_unix_ms)}`,
                  ],
                  [
                    "数量",
                    `目标 ${detail.total} · 实际处理 ${detail.completed} · 成功 ${detail.succeeded} · 失败/无效 ${detail.failed} · 跳过 ${detail.skipped} · 移除 ${detail.removed}`,
                  ],
                  [
                    "异常类别",
                    detail.error_code
                      ? (taskErrors[detail.error_code] ?? detail.error_code)
                      : "无已记录异常",
                  ],
                  ["结果引用", detail.result_ref ?? "无外部结果引用"],
                ]}
              />
              <div className="space-y-3 p-5">
                <div className="flex flex-wrap gap-4">
                  {source && (
                    <a className="text-sm text-blue-600" href={source.href}>
                      {source.label} →
                    </a>
                  )}
                  <a
                    className="text-sm text-blue-600"
                    href={`./events.html?task_id=${detail.id}&epoch=${expected ?? epoch}`}
                  >
                    该任务的结构化事件 →
                  </a>
                </div>
                {detail.result !== null && (
                  <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-all rounded-xl bg-slate-50 p-4 text-xs dark:bg-slate-800">
                    {JSON.stringify(detail.result, null, 2)}
                  </pre>
                )}
                <p className="muted text-xs leading-6">
                  结果仅保留脱敏摘要或资源引用；原始配置正文不存入任务记录。取消与重试只会在服务明确支持的阶段开放。
                </p>
              </div>
            </>
          ) : (
            <p className="muted p-5 text-sm">正在读取任务详情…</p>
          )}
        </section>
      )}
      {loading && feed.state.state !== "error" && (
        <p className="muted flex items-center gap-2 text-sm">
          <RefreshCw size={14} className="animate-spin" />
          正在读取任务…
        </p>
      )}
      <div className="grid gap-4 xl:grid-cols-2" aria-label="统一任务列表">
        {value?.records.map((task) => (
          <article
            key={task.id}
            aria-label={`任务 ${task.id}`}
            className="panel min-w-0 space-y-3 p-5"
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h2 className="text-sm font-semibold">
                #{task.id} · {taskKinds[task.kind] ?? task.kind}
              </h2>
              <Badge active={task.state === "completed" && !task.failed}>
                {taskStates[task.state] ?? task.state}
              </Badge>
            </div>
            <p className="muted break-all text-xs">
              v{task.version} · {task.phase} · {dateMs(task.created_at_unix_ms)}
              <br />
              {task.source_ref ?? "独立工作"} ·{" "}
              {task.resource_id ?? task.resource_kind ?? "无具体资源 ID"}
            </p>
            <p className="break-words text-xs">
              目标 {task.total} · 处理 {task.completed} · 成功 {task.succeeded}{" "}
              · 失败/无效 {task.failed} · 跳过 {task.skipped}
            </p>
            {task.error_code && (
              <p className="break-words text-xs text-amber-700">
                {taskErrors[task.error_code] ?? task.error_code}
              </p>
            )}
            <p className="muted break-all text-xs">
              请求：{task.request_id ?? "未记录"}
              {task.configuration_revision
                ? ` · r${task.configuration_revision}`
                : ""}
            </p>
            <button
              className="btn text-xs"
              onClick={() => show(task.id, value.epoch)}
            >
              <Search size={14} />
              查看任务详情
            </button>
          </article>
        ))}
      </div>
      {value && !loading && !value.records.length && (
        <p className="panel p-8 text-center text-sm">
          未找到符合筛选的保留任务。可按原请求 ID
          查询；无记录不等于可以安全重放已付费操作。
        </p>
      )}
      {value && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="muted text-xs">
            本页 {value.records.length} 条
            {params.has("before")
              ? ` · 登记 ID 小于 ${params.get("before")}`
              : ""}
          </p>
          <div className="flex gap-2">
            <button
              className="btn"
              disabled={!params.has("before") || loading}
              onClick={() => {
                const next = new URLSearchParams(search);
                next.delete("before");
                navigate(next);
              }}
            >
              返回最新
            </button>
            <button
              className="btn"
              disabled={value.next_before === null || loading}
              onClick={() => {
                const next = new URLSearchParams(search);
                next.set("before", String(value.next_before));
                navigate(next);
              }}
            >
              较早任务
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
