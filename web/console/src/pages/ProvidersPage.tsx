import { ConfigFields, ConfigForm } from "../components/config/ConfigFields";
import {
  providerDraft,
  providerFields,
  providerPayload,
} from "../components/config/sourceFields";
import SubscriptionsPanel from "../components/SubscriptionsPanel";
import { useEffect, useRef, useState } from "react";
import {
  Box,
  Clock,
  Database,
  Plus,
  RefreshCw,
  Save,
  Play,
} from "lucide-react";
import { Badge, Stat } from "../components/ui";
import RefillPanel, { type ControllerAction } from "../components/RefillPanel";
import { getJson, postJson } from "../platform";
import type {
  ConfigWorkspace,
  ExtractionTask,
  LeaseInfo,
  ProviderConfig,
  ProviderDetail,
  ProviderView,
  ProviderControllerStatus,
  ControllerEvaluation,
} from "../types";

const message = (cause: unknown) =>
  cause instanceof Error ? cause.message : String(cause);
const date = (value: number) => new Date(value * 1000).toLocaleString("zh-CN");
const pending = (task: ExtractionTask | null) =>
  task && ["accepted", "requesting"].includes(task.state);
const states: Record<string, string> = {
  accepted: "已受理",
  requesting: "正在提取",
  succeeded: "已入库",
  failed: "失败",
  outcome_unknown: "结果不确定",
  discarded: "配置已变更，未发布",
};
const templates: Record<string, Partial<ProviderConfig>> = {
  kuaidaili: {
    id: "kuaidaili",
    name: "快代理",
    kind: "kuaidaili",
    enabled: false,
    endpoint: "https://dps.kdlapi.com/api/getdps",
    protocol: "http",
    count: 10,
    ttl_secs: null,
    query_valid_time: true,
    auth: {
      type: "hmac_sha1",
      secret_id: "",
      secret_key: "",
    },
    min_interval_secs: 60,
    params: {},
  },
  yun_free: {
    id: "yun-free",
    name: "云代理免费列表",
    kind: "yun_free",
    enabled: false,
    endpoint: "http://www.ip3366.net/free/",
    protocol: "http",
    count: 15,
    ttl_secs: 300,
    query_valid_time: false,
    min_interval_secs: 300,
    params: { stype: "1" },
  },
  json: {
    id: "custom",
    name: "短效 API",
    kind: "json",
    enabled: false,
    endpoint: "https://vendor.example/extract",
    protocol: "http",
    count: 10,
    ttl_secs: null,
    query_valid_time: false,
    min_interval_secs: 60,
    json: {
      list: "/data/proxies",
      host: "/ip",
      port: "/port",
      ttl: "/ttl",
      expires_at: null,
      expiry_milliseconds: false,
    },
  },
};

export default function ProvidersPage({
  detailPage,
  serviceEndpoint,
  serviceEpoch,
  refreshService,
}: {
  detailPage: boolean;
  serviceEndpoint: string;
  serviceEpoch: string;
  refreshService: () => Promise<void>;
}) {
  const id = detailPage
    ? (new URLSearchParams(window.location.search).get("id") ?? "")
    : "";
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [detail, setDetail] = useState<ProviderDetail | null>(null);
  const [head, setHead] = useState(0);
  const [refill, setRefill] = useState<ProviderControllerStatus | null>(null);
  const taskRef = useRef<ExtractionTask | null>(null);

  const [loaded, setLoaded] = useState(false);
  const [editor, setEditor] = useState(() => providerDraft());
  const [editorRevision, setEditorRevision] = useState<number>();
  const [editingId, setEditingId] = useState("");
  const [task, setTask] = useState<ExtractionTask | null>(null);
  useEffect(() => {
    taskRef.current = task;
  }, [task]);
  const [leases, setLeases] = useState<LeaseInfo[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [info, setInfo] = useState("");
  const [diagnostic, setDiagnostic] = useState<unknown>(null);
  const mounted = useRef(true);
  const requestKey = `hps.extract.${serviceEndpoint}:${id}`;

  async function load(signal?: AbortSignal, replaceEditor = false) {
    const [items, workspace, item, controller] = await Promise.all([
      getJson<ProviderView[]>("api/v1/providers", signal),
      getJson<ConfigWorkspace>("api/v1/config/workspace", signal),
      detailPage && id
        ? getJson<ProviderDetail>(
            `api/v1/providers/${encodeURIComponent(id)}`,
            signal,
          )
        : Promise.resolve(null),
      detailPage && id
        ? getJson<ProviderControllerStatus>(
            `api/v1/providers/${encodeURIComponent(id)}/controller`,
            signal,
          ).catch(() => null)
        : Promise.resolve(null),
    ]);
    if (!mounted.current || signal?.aborted) return;
    setProviders(items);
    setRefill(controller);
    setLoaded(true);
    setHead(workspace.desired_revision);
    if (item) {
      setDetail(item);
      setLeases(item.leases);
      setHasMore(item.leases.length === 50);
      setTask(item.tasks[0] ?? null);
      if (replaceEditor) {
        setEditor(providerDraft(item.provider.configuration));
        setEditorRevision(workspace.desired_revision);
        setEditingId(id);
      }
    }
  }
  useEffect(() => {
    mounted.current = true;
    const controller = new AbortController();
    void load(controller.signal, true).catch((cause) => {
      if (!controller.signal.aborted) setError(message(cause));
    });
    return () => {
      mounted.current = false;
      controller.abort();
    };
    // Each MPA navigation creates an independent instance and cancels the previous one.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id, serviceEndpoint]);
  useEffect(() => {
    if (!pending(task)) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const value = await getJson<ExtractionTask>(
          `api/v1/provider-tasks/${task!.id}`,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        if (!pending(value)) {
          try {
            sessionStorage.removeItem(requestKey);
          } catch {
            /* optional recovery storage */
          }
          await load(controller.signal);
          await refreshService();
        } else {
          setTask(value);
          timer = setTimeout(() => void poll(), 500);
        }
      } catch (cause) {
        if (!controller.signal.aborted) {
          setError(message(cause));
          timer = setTimeout(() => void poll(), 2000);
        }
      }
    };
    timer = setTimeout(() => void poll(), 250);
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [task?.id, task?.state, requestKey]);
  const hasController = refill !== null;
  useEffect(() => {
    if (!detailPage || !id || !loaded || !hasController) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const status = await getJson<ProviderControllerStatus>(
          `api/v1/providers/${encodeURIComponent(id)}/controller`,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        setRefill(status);
        if (
          status.last_task_id &&
          status.last_task_id > (taskRef.current?.id ?? 0)
        ) {
          const latest = await getJson<ExtractionTask>(
            `api/v1/provider-tasks/${status.last_task_id}`,
            controller.signal,
          );
          if (controller.signal.aborted) return;
          if (latest.id >= (taskRef.current?.id ?? 0)) {
            if (!pending(latest)) await load(controller.signal);
            else setTask(latest);
          }
        }
      } catch (cause) {
        if (!controller.signal.aborted) setError(message(cause));
      }
      if (!controller.signal.aborted)
        timer = setTimeout(() => void poll(), 1500);
    };
    timer = setTimeout(() => void poll(), 1500);
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [detailPage, id, loaded, hasController, serviceEndpoint]);
  async function control(action: ControllerAction) {
    const result = await postJson<
      ProviderControllerStatus | ControllerEvaluation
    >(`api/v1/providers/${encodeURIComponent(id)}/controller/${action}`);
    if (!mounted.current) return;
    if ("status" in result) {
      setRefill(result.status);
      if (result.task) setTask(result.task);
    } else setRefill(result);
    const status = "status" in result ? result.status : result;
    setInfo(
      action === "resume" && status.paused_reason
        ? `当前仍因 ${status.paused_reason} 暂停，请先处理原因。`
        : action === "pause"
          ? "后续提取已暂停；在途请求继续完成。"
          : action === "resume"
            ? "已确认并恢复；预算和冷却保持不变。"
            : "补池评估已完成，请查看实际决策和任务。",
    );
  }
  function openRefillSettings() {
    document
      .querySelector<HTMLDetailsElement>(
        '#provider-editor details[data-config-key="refill"]',
      )
      ?.setAttribute("open", "");
    document
      .getElementById("provider-editor")
      ?.scrollIntoView({ block: "start" });
  }
  async function act(key: string, action: () => Promise<void>) {
    setBusy(key);
    setError("");
    setInfo("");
    try {
      await action();
    } catch (cause) {
      if (mounted.current) setError(message(cause));
    } finally {
      if (mounted.current) setBusy("");
    }
  }
  async function save(configuration: ProviderConfig, revision = head) {
    const result = await postJson<{ state: string }>(
      `api/v1/providers/${encodeURIComponent(configuration.id)}/save`,
      { configuration, expected_revision: revision },
    );
    if (!mounted.current) return;
    setInfo(
      result.state === "applied"
        ? "来源配置已激活。新提取使用当前配置，旧请求不会越过配置指纹发布。"
        : "来源配置已保存，等待重启。",
    );
    await load(undefined, detailPage);
    await refreshService();
  }
  function resetEditor() {
    setEditor(providerDraft());
    setEditingId("");
    setEditorRevision(undefined);
  }
  function template(kind: string) {
    setEditor(providerDraft(templates[kind]));
    setEditorRevision(head);
    setEditingId("");
    setInfo("");
  }
  async function extract() {
    let requestId: string | null = null;
    try {
      requestId = sessionStorage.getItem(requestKey);
    } catch {
      /* optional storage */
    }
    requestId ??= crypto.randomUUID();
    try {
      sessionStorage.setItem(requestKey, requestId);
    } catch {
      /* server still merges in-flight requests */
    }
    const accepted = await postJson<ExtractionTask>(
      `api/v1/providers/${encodeURIComponent(id)}/extract`,
      { request_id: requestId },
    );
    if (!mounted.current) return;
    setTask(accepted);
    setInfo(
      `提取任务 #${accepted.id}：${states[accepted.state] ?? accepted.state}。离开页面后仍由服务继续处理。`,
    );
    if (!pending(accepted)) {
      try {
        sessionStorage.removeItem(requestKey);
      } catch {
        /* optional storage */
      }
      await load();
    }
  }
  if (!loaded && !error)
    return <div className="panel p-8 text-sm muted">正在读取供应商配置…</div>;
  if (!loaded)
    return (
      <div
        role="alert"
        className="panel panel-body text-sm text-console-danger"
      >
        {error}
        <button
          className="btn ml-3"
          onClick={() => void act("reload", () => load(undefined, true))}
        >
          重新读取服务
        </button>
      </div>
    );
  const current = detail?.provider.configuration;
  return (
    <div className="space-y-3">
      {error && (
        <div
          role="alert"
          className="panel panel-body break-words border-console-danger-border text-sm text-console-danger"
        >
          {error}
          <button
            className="btn ml-3"
            onClick={() => void act("reload", () => load(undefined, true))}
          >
            重新读取服务
          </button>
        </div>
      )}
      {info && (
        <p role="status" className="text-sm text-console-accent-ink">
          {info}
        </p>
      )}
      {!detailPage && (
        <SubscriptionsPanel
          refreshService={refreshService}
          onRevision={() => load()}
        />
      )}
      <div className="metric-grid">
        <Stat
          title="来源数量"
          value={String(providers.length)}
          detail="配置来源，非演示节点"
          icon={Box}
        />
        <Stat
          title="有效租约记录"
          value={String(providers.reduce((sum, p) => sum + p.active_leases, 0))}
          detail="来源启用且匹配配置；尚不代表健康通过"
          icon={Database}
        />
        <Stat
          title="期望配置"
          value={`r${head}`}
          detail="来源修改使用配置版本冲突检查"
          icon={Save}
        />
        <Stat
          title="安全提取"
          value="不自动重放"
          detail="结果不确定时保守记录，不重复消耗额度"
          icon={Clock}
        />
      </div>
      {detailPage ? (
        <>
          <div className="panel panel-body">
            <a
              className="text-sm text-console-accent-ink"
              href="./providers.html"
            >
              ← 全部来源
            </a>
            <h2 className="mt-4 text-lg font-semibold">
              {current?.name || id || "请从来源列表选择"}
            </h2>
            {current && (
              <>
                <div className="mt-3 flex flex-wrap items-center gap-3">
                  <Badge active={current.enabled}>
                    {current.enabled ? "已启用" : "已停用"}
                  </Badge>
                  <span className="muted text-sm">
                    {current.kind} · {current.protocol} · 最小间隔{" "}
                    {current.min_interval_secs} 秒
                  </span>
                </div>
                <p className="muted mt-4 text-sm leading-6">
                  本次请求最多提取 {current.count}{" "}
                  个节点，可能消耗供应商额度。重复点击合并在途任务；超时、断线或结果不确定不会自动重新购买。TTL
                  从请求开始时保守起算，免费源期限仅为本地缓存上限。
                </p>
                <div className="mt-4 flex flex-wrap gap-2">
                  <button
                    className="btn"
                    disabled={!!busy || !current.enabled || !!pending(task)}
                    onClick={() => void act("extract", extract)}
                  >
                    <Play size={15} />
                    提取 {current.count} 个节点
                  </button>
                  <button
                    className="btn"
                    disabled={!!busy || !current.enabled}
                    onClick={() =>
                      void act("quota", async () =>
                        setDiagnostic(
                          await postJson(
                            `api/v1/providers/${encodeURIComponent(id)}/diagnose/quota`,
                          ),
                        ),
                      )
                    }
                  >
                    查询额度
                  </button>
                  <button
                    className="btn"
                    disabled={!!busy || !current.enabled}
                    onClick={() =>
                      void act("whitelist", async () =>
                        setDiagnostic(
                          await postJson(
                            `api/v1/providers/${encodeURIComponent(id)}/diagnose/whitelist`,
                          ),
                        ),
                      )
                    }
                  >
                    查询供应商白名单
                  </button>
                  <button
                    className="btn"
                    disabled={!!busy}
                    onClick={() => void act("reload", () => load())}
                  >
                    <RefreshCw size={15} />
                    刷新来源
                  </button>
                </div>
              </>
            )}
            {task && (
              <div
                className="mt-4 rounded-lg bg-console-soft p-4 text-sm"
                data-testid="extraction-result"
              >
                <strong>
                  任务 #{task.id} · {states[task.state] ?? task.state}
                </strong>
                <p className="mt-2 break-all text-xs">
                  request_id: {task.request_id}
                </p>
                <a
                  className="mt-2 inline-flex text-xs text-console-accent-ink"
                  href={`./tasks.html?source_ref=provider:${task.id}&epoch=${encodeURIComponent(serviceEpoch)}`}
                >
                  查看统一任务与关联事件 →
                </a>
                <p className="mt-2">
                  新增 {task.added} · 拒收 {task.rejected} · 撤销 {task.removed}
                  {task.error_code ? ` · ${task.error_code}` : ""}
                </p>
                {task.state === "outcome_unknown" && (
                  <p className="mt-2 text-console-warning">
                    本次提取可能已扣费。没有自动重试；再次发起新请求前请检查供应商记录。
                  </p>
                )}
              </div>
            )}
            {diagnostic !== null && (
              <pre className="code-block mt-4">
                {JSON.stringify(diagnostic, null, 2)}
              </pre>
            )}
          </div>
          {refill && (
            <RefillPanel
              value={refill}
              busy={!!busy}
              onAction={(action) => void act(action, () => control(action))}
              onConfigure={openRefillSettings}
            />
          )}
          <div className="panel overflow-hidden">
            <h2 className="panel-header">
              批次与租约{" "}
              <a
                className="ml-3 text-xs font-normal text-console-accent-ink"
                href={`./nodes.html?provider=${encodeURIComponent(id)}`}
              >
                查看来源节点 →
              </a>
            </h2>
            <div className="overflow-x-auto">
              <table className="w-full min-w-[850px] text-left text-sm">
                <thead className="muted bg-console-soft text-xs">
                  <tr>
                    {[
                      "租约 / 批次",
                      "端点",
                      "状态 / 地域",
                      "接收 / 到期",
                      "有效期依据",
                    ].map((label) => (
                      <th key={label} className="p-4">
                        {label}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {leases.map((lease) => (
                    <tr
                      key={lease.id}
                      className="border-t border-console-border"
                    >
                      <td className="p-4">
                        <a
                          className="text-console-accent-ink"
                          href={`./node-detail.html?id=${encodeURIComponent(lease.upstream_id)}`}
                        >
                          #{lease.id} / #{lease.batch_id}
                        </a>
                      </td>
                      <td className="p-4 font-mono text-xs">
                        {lease.protocol}://
                        {lease.host.includes(":")
                          ? `[${lease.host}]`
                          : lease.host}
                        :{lease.port}
                      </td>
                      <td className="p-4">
                        {lease.expires_at <= Date.now() / 1000 &&
                        lease.state === "active"
                          ? "expired"
                          : lease.state}
                        <p className="muted text-xs">
                          {lease.region ?? "未知地域"}
                        </p>
                      </td>
                      <td className="p-4 text-xs leading-6">
                        {date(lease.received_at)}
                        <br />
                        {date(lease.expires_at)}
                      </td>
                      <td className="max-w-64 break-words p-4 text-xs">
                        {lease.expiry_basis}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {leases.length === 0 && (
              <p className="muted p-6 text-sm">
                当前没有租约记录。抓取成功与代理健康分开判断。
              </p>
            )}
            {hasMore && (
              <button
                className="btn m-4"
                disabled={!!busy}
                onClick={() =>
                  void act("more", async () => {
                    const next = await getJson<LeaseInfo[]>(
                      `api/v1/providers/${encodeURIComponent(id)}/leases?before=${leases[leases.length - 1].id}&limit=50`,
                    );
                    if (mounted.current) {
                      setLeases(next);
                      setHasMore(next.length === 50);
                    }
                  })
                }
              >
                查看更早租约（每页 50 条）
              </button>
            )}
          </div>
          {detail && (
            <div className="panel panel-body">
              <h2 className="font-semibold">最近 50 个提取任务</h2>
              <ul className="mt-4 space-y-3 text-sm">
                {detail.tasks.map((item) => (
                  <li
                    key={item.id}
                    className="flex flex-wrap justify-between gap-2 border-b border-console-border pb-3"
                  >
                    <span>
                      #{item.id} · {states[item.state] ?? item.state} · 新增{" "}
                      {item.added}
                    </span>
                    <span className="muted text-xs">
                      {date(item.started_at)}
                      {item.error_code ? ` · ${item.error_code}` : ""}
                    </span>
                    <a
                      className="text-xs text-console-accent-ink"
                      href={`./tasks.html?source_ref=provider:${item.id}&epoch=${encodeURIComponent(serviceEpoch)}`}
                    >
                      统一任务 →
                    </a>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </>
      ) : (
        <div className="panel overflow-hidden">
          <div className="panel-header panel-toolbar">
            <h2 className="font-semibold">供应商来源</h2>
            <button
              className="btn"
              disabled={!!busy}
              onClick={() => void act("reload", () => load())}
            >
              <RefreshCw size={15} />
              刷新来源
            </button>
          </div>
          {providers.length === 0 && (
            <p className="muted py-10 text-center text-sm">
              尚未配置供应商。选择下方模板，填写接口与有效期后保存。
            </p>
          )}
          <div>
            {providers.map((view) => (
              <div key={view.configuration.id} className="management-record">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <a
                    className="min-w-0 break-all text-sm font-semibold text-console-accent-ink"
                    href={`./provider-detail.html?id=${encodeURIComponent(view.configuration.id)}`}
                  >
                    {view.configuration.name || view.configuration.id}
                  </a>
                  <Badge active={view.configuration.enabled}>
                    {view.configuration.enabled ? "启用" : "停用"}
                  </Badge>
                </div>
                <div className="min-w-0 text-xs">
                  <p>
                    {view.active_leases} 条有效租约 · {view.configuration.kind}
                  </p>
                  <p
                    className="muted truncate"
                    title={view.configuration.endpoint}
                  >
                    {view.configuration.endpoint}
                  </p>
                </div>
                <div className="flex flex-wrap gap-2">
                  <a
                    className="btn"
                    href={`./provider-detail.html?id=${encodeURIComponent(view.configuration.id)}`}
                  >
                    详情与提取
                  </a>
                  <button
                    className="btn"
                    disabled={!!busy}
                    onClick={() =>
                      void act(view.configuration.id, () =>
                        save({
                          ...view.configuration,
                          enabled: !view.configuration.enabled,
                        }),
                      )
                    }
                  >
                    {view.configuration.enabled ? "停用" : "启用"}
                  </button>
                  <button
                    className="btn"
                    disabled={!!busy}
                    onClick={() => {
                      setEditor(providerDraft(view.configuration));
                      setEditorRevision(head);
                      setEditingId(view.configuration.id);
                      document
                        .getElementById("provider-editor")
                        ?.scrollIntoView({ block: "start" });
                    }}
                  >
                    编辑
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
      <div id="provider-editor" className="panel panel-body">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="font-semibold">
            {editingId ? `编辑来源 ${editingId}` : "新增来源"}
          </h2>
          <div className="flex flex-wrap gap-2">
            {Object.entries({
              kuaidaili: "快代理模板",
              yun_free: "云代理模板",
              json: "通用 JSON 模板",
            }).map(([kind, label]) => (
              <button
                className="btn"
                key={kind}
                disabled={!!busy}
                onClick={() => template(kind)}
              >
                <Plus size={14} />
                {label}
              </button>
            ))}
          </div>
        </div>
        <p className="muted mt-3 text-xs leading-6">
          API 地址填写不带查询参数的完整
          URL，鉴权和请求参数在下方配置。保留脱敏值可复用同 ID 的既有凭据。
        </p>
        <ConfigForm
          label="供应商配置"
          className="mt-3 space-y-3"
          onSave={() => {
            if (busy || !head) return;
            void act("save", async () => {
              await save(providerPayload(editor), editorRevision ?? head);
              if (!detailPage) resetEditor();
            });
          }}
        >
          <ConfigFields
            fields={providerFields(Boolean(editingId), editor)}
            value={editor}
            onChange={(value) => {
              const auth = value.auth?.type;
              setEditor(
                auth && auth !== editor.auth?.type
                  ? { ...value, auth: { type: auth } }
                  : value,
              );
              setEditorRevision((revision) => revision ?? head);
            }}
          />
          <div className="flex flex-wrap gap-2">
            <button
              className="btn-primary"
              type="submit"
              disabled={!!busy || !head}
            >
              <Save size={15} />
              保存并激活来源
            </button>
            {editingId && (
              <button
                className="btn text-console-danger"
                type="button"
                disabled={!!busy}
                onClick={() =>
                  void act("delete", async () => {
                    await postJson(
                      `api/v1/providers/${encodeURIComponent(editingId)}/delete`,
                      { expected_revision: head },
                    );
                    if (detailPage) window.location.assign("./providers.html");
                    else {
                      resetEditor();
                      await load();
                      await refreshService();
                    }
                  })
                }
              >
                删除来源并停止新准入
              </button>
            )}
            <a href="./config.html" className="btn">
              打开配置工作台
            </a>
          </div>
        </ConfigForm>
        <p className="muted mt-3 text-xs">
          删除会保留租约和批次历史，并停止该来源的新选点。已有连接继续排空。
        </p>
      </div>
    </div>
  );
}
