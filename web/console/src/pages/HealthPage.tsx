import { useEffect, useRef, useState } from "react";
import {
  Activity,
  Clock,
  ListChecks,
  Play,
  RefreshCw,
  Save,
  Settings2,
} from "lucide-react";
import { Badge, Stat } from "../components/ui";
import { getJson, postJson } from "../platform";
import type {
  HealthConfig,
  HealthOverview,
  HealthTask,
  ProbeResult,
} from "../types";
const verdicts: Record<string, string> = {
  unknown: "未确认",
  queued: "排队中",
  probing: "检测中",
  healthy: "可达",
  degraded: "降级",
  failed: "失败",
  unsupported: "暂不支持",
  expired: "已过期",
  skipped: "已跳过",
};
const phases: Record<string, string> = {
  target_resolve: "目标解析",
  endpoint_connect: "端点连接",
  upstream_tls: "上游 TLS",
  authentication: "代理认证",
  proxy_tunnel: "代理隧道",
  target_tls: "目标 TLS",
  target_http: "目标 HTTP",
  complete: "完成",
  admission: "寿命与准入",
  local: "本地检测",
  unsupported: "能力不支持",
};
const date = (value: number) =>
  value ? new Date(value * 1000).toLocaleString("zh-CN") : "尚未检测";
const message = (error: unknown) =>
  error instanceof Error ? error.message : String(error);
interface Pending {
  request_id: string;
  node_ids: string[];
}
function Evidence({ result }: { result: ProbeResult }) {
  return (
    <div className="rounded-xl border border-slate-200 p-4 text-xs dark:border-slate-700">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <strong>
          {result.transport.toUpperCase()} / {result.family.toUpperCase()}
        </strong>
        <Badge active={result.verdict === "healthy"}>
          {verdicts[result.verdict]}
        </Badge>
      </div>
      <dl className="mt-4 grid grid-cols-2 gap-y-2 leading-5">
        <dt className="muted">最近检测</dt>
        <dd>{date(result.checked_at)}</dd>
        <dt className="muted">阶段 / 耗时</dt>
        <dd>
          {phases[result.phase] ?? result.phase} ·{" "}
          {result.checked_at ? `${result.latency_ms} ms` : "—"}
        </dd>
        <dt className="muted">端点 / 认证 / 隧道</dt>
        <dd>
          {result.endpoint_reachable ? "已连接" : "未证实"} /{" "}
          {result.authenticated === null
            ? "—"
            : result.authenticated
              ? "通过"
              : "失败"}{" "}
          / {result.tunnel_established ? "已建立" : "—"}
        </dd>
        <dt className="muted">目标响应</dt>
        <dd>
          {result.http_status ?? "—"}
          {result.target_ok === false ? " · 目标未接受" : ""}
        </dd>
      </dl>
      {result.error_code && (
        <p className="mt-3 break-all rounded-lg bg-slate-50 p-2 font-mono leading-5 dark:bg-slate-800">
          {result.error_code}
        </p>
      )}
    </div>
  );
}
export default function HealthPage({
  serviceEndpoint,
  serviceEpoch,
  refreshService,
}: {
  serviceEndpoint: string;
  serviceEpoch: string;
  refreshService: () => Promise<void>;
}) {
  const [data, setData] = useState<HealthOverview | null>(null);
  const nodeFilter =
    new URLSearchParams(window.location.search).get("node") ?? "";
  const [after, setAfter] = useState("");
  const [selected, setSelected] = useState<string[]>(
    nodeFilter ? [nodeFilter] : [],
  );
  const [expanded, setExpanded] = useState(nodeFilter);
  const [task, setTask] = useState<HealthTask | null>(null);
  const linked = new URLSearchParams(window.location.search);
  const linkedTask = linked.get("task") ?? "";
  const taskId = useRef<number | null>(
    /^[1-9]\d*$/.test(linkedTask) ? Number(linkedTask) : null,
  );
  const taskEpoch = useRef(linked.get("epoch") ?? serviceEpoch);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [info, setInfo] = useState("");
  const [editor, setEditor] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [pending, setPending] = useState<Pending | null>(null);
  const storageKey = `hps.health.${serviceEndpoint}`;
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    try {
      const value = sessionStorage.getItem(storageKey);
      if (value) {
        const parsed = JSON.parse(value) as Pending;
        if (
          typeof parsed.request_id === "string" &&
          Array.isArray(parsed.node_ids)
        )
          setPending(parsed);
      }
    } catch {
      /* Recovery is optional. */
    }
    return () => {
      mounted.current = false;
    };
  }, [storageKey]);
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const value = await getJson<HealthOverview>(
          `api/v1/health?limit=50${nodeFilter ? `&node=${encodeURIComponent(nodeFilter)}` : ""}${after ? `&after=${encodeURIComponent(after)}` : ""}`,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        setData(value);
        setLoadError("");
        if (taskId.current) {
          const detail = await getJson<HealthTask>(
            `api/v1/health/tasks/${taskId.current}?expected_epoch=${encodeURIComponent(taskEpoch.current)}`,
            controller.signal,
          );
          if (!controller.signal.aborted) setTask(detail);
        }
      } catch (cause) {
        if (!controller.signal.aborted) setLoadError(message(cause));
      } finally {
        if (!controller.signal.aborted)
          timer = setTimeout(() => void poll(), 2000);
      }
    }
    void poll();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [after]);
  async function refresh() {
    const value = await getJson<HealthOverview>(
      `api/v1/health?limit=50${nodeFilter ? `&node=${encodeURIComponent(nodeFilter)}` : ""}${after ? `&after=${encodeURIComponent(after)}` : ""}`,
    );
    if (mounted.current) setData(value);
  }
  async function action(name: string, run: () => Promise<void>) {
    setBusy(name);
    setError("");
    setInfo("");
    try {
      await run();
    } catch (cause) {
      if (mounted.current) setError(message(cause));
    } finally {
      if (mounted.current) setBusy("");
    }
  }
  async function probe(node_ids: string[], recovery?: Pending) {
    const input = recovery ?? { request_id: crypto.randomUUID(), node_ids };
    setPending(input);
    try {
      sessionStorage.setItem(storageKey, JSON.stringify(input));
    } catch {
      /* The request ID remains visible. */
    }
    const value = await postJson<HealthTask>("api/v1/health/probe", input);
    if (!mounted.current) return;
    setTask(value);
    taskId.current = value.id;
    taskEpoch.current = serviceEpoch;
    setPending(null);
    try {
      sessionStorage.removeItem(storageKey);
    } catch {
      /* Optional. */
    }
    setInfo(`检测任务 health:${value.id} 已受理，切换页面后仍在服务端运行。`);
    await refresh();
  }
  async function openEditor() {
    const snapshot = await getJson<HealthOverview>("api/v1/health?limit=1");
    if (mounted.current) {
      setRevision(snapshot.running_revision);
      setEditor(JSON.stringify(snapshot.configuration, null, 2));
    }
  }
  async function save() {
    const configuration = JSON.parse(editor ?? "") as HealthConfig;
    const result = await postJson<{ state: string; running_revision: number }>(
      "api/v1/health/save",
      { configuration, expected_revision: revision },
    );
    if (!mounted.current) return;
    setEditor(null);
    setInfo(
      `检测策略已保存 · ${result.state} · 运行版本 r${result.running_revision}`,
    );
    await Promise.all([refresh(), refreshService()]);
  }
  return (
    <div className="space-y-6">
      {(error || loadError) && (
        <div
          role="alert"
          className="rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-200"
        >
          {error || loadError}
        </div>
      )}
      {info && (
        <p
          role="status"
          className="rounded-xl bg-blue-50 p-4 text-sm text-blue-700 dark:bg-blue-950 dark:text-blue-200"
        >
          {info}
        </p>
      )}
      {nodeFilter && (
        <p className="text-sm">
          节点筛选：{nodeFilter}{" "}
          <a className="ml-2 text-blue-600" href="./health.html">
            查看全部健康记录 →
          </a>
        </p>
      )}
      {!data ? (
        <div className="panel p-10 text-center text-sm">正在读取健康状态…</div>
      ) : (
        <>
          <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
            <Stat
              title="本页可准入"
              value={`${data.nodes.filter((n) => n.eligible).length} / ${data.nodes.length}`}
              detail={`当前池共 ${data.total_nodes} 个节点`}
              icon={Activity}
            />
            <Stat
              title="探测队列"
              value={`${data.queued} / ${data.probing}`}
              detail={`排队 / 检测中 · 最大并发 ${data.configuration.max_parallel}`}
              icon={ListChecks}
            />
            <Stat
              title="UTC 每日探测预算"
              value={`${data.used_probes} / ${data.budget}`}
              detail={`重置：${new Date(data.resets_at * 1000).toISOString().slice(0, 16).replace("T", " ")} UTC`}
              icon={Clock}
            />
            <Stat
              title="检测周期"
              value={`${data.configuration.interval_secs} s`}
              detail={`证据最长保留 ${data.configuration.max_staleness_secs} s · ${data.configuration.enabled ? "检测已启用" : "检测已显式停用"}`}
              icon={RefreshCw}
            />
          </div>
          <section className="panel p-5">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <h2 className="font-semibold">检测策略</h2>
              <button
                className="btn"
                disabled={!!busy}
                onClick={() => void action("edit", openEditor)}
              >
                <Settings2 size={15} />
                编辑检测策略
              </button>
            </div>
            <div className="mt-4 space-y-2 text-sm">
              {data.configuration.profiles.map((profile) => (
                <p
                  className="break-all"
                  key={`${profile.transport}/${profile.family}`}
                >
                  <span className="mr-3 font-medium">
                    {profile.transport.toUpperCase()} /{" "}
                    {profile.family.toUpperCase()}
                  </span>
                  <span className="muted">
                    {profile.enabled ? profile.url : "停用"}
                  </span>
                </p>
              ))}
            </div>
            <p className="muted mt-4 text-xs leading-6">
              HTTP、HTTPS、SOCKS5
              入口共用这些状态。端口连通、认证、隧道和目标响应分别记录；403/429
              保留可达证据。UDP 能力尚不支持时单独显示。
            </p>
            {editor !== null && (
              <div className="mt-5 space-y-3">
                <label
                  htmlFor="health-policy"
                  className="block text-sm font-medium"
                >
                  检测策略 JSON · 基于配置 r{revision}
                </label>
                <textarea
                  id="health-policy"
                  className="input min-h-80 w-full font-mono text-xs"
                  value={editor}
                  onChange={(event) => setEditor(event.target.value)}
                  spellCheck={false}
                />
                <p className="muted text-xs">
                  服务端验证策略和 TLS
                  信任后应用。其他页面已修改配置时会提示冲突，请重新打开编辑器。
                </p>
                <div className="flex gap-2">
                  <button
                    className="btn-primary"
                    disabled={!!busy}
                    onClick={() => void action("save", save)}
                  >
                    <Save size={15} />
                    保存并应用策略
                  </button>
                  <button
                    className="btn"
                    disabled={!!busy}
                    onClick={() => setEditor(null)}
                  >
                    取消编辑
                  </button>
                </div>
              </div>
            )}
          </section>
          <section className="panel overflow-hidden" aria-label="节点健康状态">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-100 p-5 dark:border-slate-800">
              <h2 className="font-semibold">节点准入与能力</h2>
              <div className="flex flex-wrap gap-2">
                <button
                  className="btn"
                  disabled={!!busy}
                  onClick={() => void action("refresh", refresh)}
                >
                  <RefreshCw size={15} />
                  刷新
                </button>
                <button
                  className="btn-primary"
                  disabled={
                    !!busy || !data.configuration.enabled || !data.nodes.length
                  }
                  onClick={() =>
                    void action("batch", () =>
                      probe(
                        selected.length
                          ? selected
                          : data.nodes.map((n) => n.node_id),
                      ),
                    )
                  }
                >
                  <Play size={15} />
                  {selected.length
                    ? `检测所选 ${selected.length} 个`
                    : `检测本页 ${data.nodes.length} 个`}
                </button>
              </div>
            </div>
            {pending && (
              <div className="m-5 rounded-xl bg-amber-50 p-4 text-xs leading-6 text-amber-800 dark:bg-amber-950 dark:text-amber-200">
                <p>上次检测受理结果尚未确认 · {pending.request_id}</p>
                <button
                  className="btn mt-2"
                  disabled={!!busy}
                  onClick={() =>
                    void action("recover", () =>
                      probe(pending.node_ids, pending),
                    )
                  }
                >
                  恢复上次检测
                </button>
              </div>
            )}
            {!data.nodes.length ? (
              <p className="muted p-10 text-center text-sm">
                当前池暂无节点。先在供应商页面提取节点或添加静态上游。
              </p>
            ) : (
              <div className="divide-y divide-slate-100 dark:divide-slate-800">
                {data.nodes.map((node) => (
                  <div key={node.node_id}>
                    <div className="flex flex-wrap items-center gap-3 p-5">
                      <input
                        type="checkbox"
                        aria-label={`选择节点 ${node.node_id}`}
                        checked={selected.includes(node.node_id)}
                        onChange={(event) =>
                          setSelected((previous) =>
                            event.target.checked
                              ? [...previous, node.node_id]
                              : previous.filter((id) => id !== node.node_id),
                          )
                        }
                      />
                      <div className="min-w-0 flex-1 basis-[calc(100%_-_2rem)] sm:basis-0">
                        <button
                          className="break-all text-left text-sm font-semibold hover:text-blue-600"
                          aria-expanded={expanded === node.node_id}
                          onClick={() =>
                            setExpanded(
                              expanded === node.node_id ? "" : node.node_id,
                            )
                          }
                        >
                          {node.node_id} · {node.host}:{node.port}
                        </button>
                        <p className="muted mt-1 text-xs">
                          {node.protocol.toUpperCase()} ·{" "}
                          {node.provider_id ?? "静态上游"} ·{" "}
                          {node.remaining_secs === null
                            ? "无租约期限"
                            : `剩余 ${node.remaining_secs} s`}
                        </p>
                      </div>
                      <Badge active={node.eligible}>
                        {node.eligible
                          ? data.configuration.enabled
                            ? "可准入"
                            : "检测停用，允许准入"
                          : node.enabled
                            ? "等待可用证据"
                            : "已停用"}
                      </Badge>
                      <button
                        className="btn"
                        disabled={
                          !!busy || !data.configuration.enabled || !node.enabled
                        }
                        onClick={() =>
                          void action(node.node_id, () => probe([node.node_id]))
                        }
                      >
                        单个检测
                      </button>
                    </div>
                    {expanded === node.node_id && (
                      <div className="grid gap-3 px-5 pb-5 md:grid-cols-2">
                        {node.last_traffic && (
                          <div className="rounded-xl bg-slate-50 p-4 text-xs leading-6 dark:bg-slate-800 md:col-span-2">
                            <strong>当前进程内最近流量反馈</strong>
                            <p>
                              {date(node.last_traffic.observed_at)} ·{" "}
                              {phases[node.last_traffic.phase] ??
                                node.last_traffic.phase}{" "}
                              ·{" "}
                              {node.last_traffic.invalidated
                                ? "已阻止后续选点"
                                : "保留分层判断"}
                            </p>
                            <p className="break-all font-mono">
                              {node.last_traffic.error_code}
                            </p>
                            <p className="muted">
                              正常业务响应不延长探测证据的有效时间。
                            </p>
                          </div>
                        )}
                        {node.results.map((result) => (
                          <Evidence
                            key={`${result.transport}/${result.family}`}
                            result={result}
                          />
                        ))}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
            <div className="flex justify-between border-t border-slate-100 p-4 dark:border-slate-800">
              <button
                className="btn"
                disabled={!after || !!busy}
                onClick={() => {
                  setAfter("");
                  setSelected([]);
                }}
              >
                返回首页
              </button>
              <button
                className="btn"
                disabled={!data.next_after || !!busy}
                onClick={() => {
                  setAfter(data.next_after ?? "");
                  setSelected([]);
                }}
              >
                下一页
              </button>
            </div>
          </section>
          <section className="panel p-5" aria-label="健康检测任务">
            <h2 className="font-semibold">后台检测任务</h2>
            <div className="mt-4 grid gap-4 lg:grid-cols-2">
              <div className="max-h-96 space-y-2 overflow-y-auto">
                {data.tasks.length ? (
                  data.tasks.map((item) => (
                    <button
                      key={item.id}
                      className="flex w-full items-center justify-between rounded-xl border border-slate-200 p-3 text-left text-xs hover:border-blue-400 dark:border-slate-700"
                      onClick={() =>
                        void action("task", async () => {
                          const value = await getJson<HealthTask>(
                            `api/v1/health/tasks/${item.id}?expected_epoch=${encodeURIComponent(serviceEpoch)}`,
                          );
                          if (mounted.current) {
                            taskId.current = value.id;
                            taskEpoch.current = serviceEpoch;
                            setTask(value);
                          }
                        })
                      }
                    >
                      <span>
                        <strong>health:{item.id}</strong> ·{" "}
                        {item.trigger === "manual" ? "手动" : "自动"}
                        <span className="muted mt-1 block">
                          {date(item.created_at)}
                        </span>
                      </span>
                      <span>
                        {item.completed} / {item.total} · {item.state}
                      </span>
                    </button>
                  ))
                ) : (
                  <p className="muted text-sm">暂无检测任务</p>
                )}
              </div>
              <div className="min-w-0">
                {task ? (
                  <>
                    <h3 className="text-sm font-semibold">
                      health:{task.id} · {task.completed}/{task.total} ·{" "}
                      {task.state}
                    </h3>
                    <p className="muted mt-2 break-all text-xs">
                      请求 ID：{task.request_id}
                    </p>
                    <a
                      className="mt-2 inline-flex text-xs text-blue-600"
                      href={`./tasks.html?source_ref=health:${task.id}&epoch=${encodeURIComponent(taskEpoch.current)}`}
                    >
                      查看统一任务与关联事件 →
                    </a>
                    <p className="mt-3 text-xs">
                      可达 {task.healthy} · 失败 {task.failed} · 跳过{" "}
                      {task.skipped} · 合并在途 {task.merged}
                    </p>
                    <div className="mt-4 max-h-80 space-y-2 overflow-y-auto">
                      {task.runs.slice(0, 100).map((run) => (
                        <div
                          key={run.id}
                          className="rounded-lg bg-slate-50 p-3 text-xs leading-6 dark:bg-slate-800"
                        >
                          <strong>{run.node_id}</strong> · {run.state}
                          <p className="muted break-all">
                            {run.result
                              ? `${verdicts[run.result.verdict]} · ${phases[run.result.phase]} · ${run.result.error_code ?? run.result.http_status ?? "完成"}`
                              : "等待执行或已中断"}
                          </p>
                        </div>
                      ))}
                    </div>
                    {task.runs.length > 100 && (
                      <p className="muted mt-2 text-xs">
                        此处显示前 100 条，全部结果可通过 CLI 或 API 获取。
                      </p>
                    )}
                  </>
                ) : (
                  <p className="muted text-sm">
                    选择任务查看进度和分层结果。任务存储在服务端，切页不会取消。
                  </p>
                )}
              </div>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
