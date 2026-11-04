import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  Activity,
  RefreshCw,
  Pause,
  Play,
  Filter,
  Unplug,
  Search,
  X,
} from "lucide-react";
import { AdvancedFilters, FilterStartTime } from "../components/FilterOptions";
import { ApiRequestError, getJson, postJson } from "../platform";
import { Badge, Details } from "../components/ui";
import { ConnectionStats } from "../components/ConnectionSummary";
import ObservationPanel from "../components/ObservationPanel";
import { candidateReasons, strategyLabels } from "../selection";
import {
  connectionStates,
  connectionErrors,
  endpointLabel,
  formatBytes,
  sampleRates,
  type ConnectionInfo,
  type ConnectionStatistics,
  type ConnectionsOverview,
  type DisconnectPreview,
  type DisconnectResult,
  type Rates,
} from "../connections";

const message = (cause: unknown) =>
  cause instanceof Error ? cause.message : String(cause);
const date = (value: number | null) =>
  value === null ? "尚未结束" : new Date(value).toLocaleString("zh-CN");
const filterKeys = [
  "id",
  "source",
  "source_port",
  "destination",
  "destination_port",
  "inbound",
  "protocol",
  "transport",
  "origin",
  "node",
  "provider",
  "group",
  "rule",
  "state",
  "error",
  "created_after_unix_ms",
  "sort",
  "offset",
  "limit",
];
const recoverable = new Set([
  "connection_snapshot_expired",
  "connection_snapshot_filters_changed",
  "connection_instance_changed",
]);
const domainSources: Record<string, string> = {
  explicit: "原始代理目标域名",
  dns_observation: "有效的单一 DNS 观测",
  ambiguous_dns_observations: "多个 DNS 域名，不能唯一归属",
  unknown: "无可用域名上下文",
};
const origins: Record<string, string> = {
  explicit: "显式代理",
  linux_local: "Linux 本机",
  gateway: "网关",
};
const active = (record: ConnectionInfo) => record.ended_at_unix_ms === null;
function stateLabel(record: ConnectionInfo) {
  const label = connectionStates[record.state] ?? record.state;
  return active(record) && ["failed", "rejected"].includes(record.state)
    ? `${label}，正在回收`
    : label;
}

export default function ConnectionsPage() {
  const [query, setQuery] = useState(window.location.search);
  const [reload, setReload] = useState(0);
  const [automatic, setAutomatic] = useState(true);
  const [value, setValue] = useState<ConnectionsOverview | null>(null);
  const [rates, setRates] = useState<Rates | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);
  const [preview, setPreview] = useState<DisconnectPreview | null>(null);
  const [single, setSingle] = useState<ConnectionInfo | null>(null);
  const [detail, setDetail] = useState<ConnectionInfo | null>(null);
  const [detailError, setDetailError] = useState("");
  const snapshot = useRef<{ id: string; epoch: string; scope: string } | null>(
    null,
  );
  const previous = useRef<ConnectionStatistics | null>(null);
  const lastEpoch = useRef<string | null>(null);
  const intent = useRef(0);
  const lastReload = useRef(-1);
  const params = new URLSearchParams(query);
  const pageSizes = Array.from(
    new Set([20, 50, 100, 200, Number(params.get("limit") ?? 50)]),
  )
    .filter((n) => Number.isInteger(n) && n >= 1 && n <= 200)
    .sort((a, b) => a - b);
  const detailId = params.get("detail") ?? params.get("id");
  const detailRegion = useRef<HTMLElement>(null);
  useEffect(() => {
    if (detailId) detailRegion.current?.focus();
  }, [detailId]);
  const apiParams = new URLSearchParams();
  for (const key of filterKeys) {
    const v = params.get(key);
    if (v) apiParams.set(key, v);
  }
  const apiQuery = apiParams.toString();
  const scopeParams = new URLSearchParams(apiParams);
  scopeParams.delete("offset");
  scopeParams.delete("limit");
  scopeParams.sort();
  const scope = scopeParams.toString();
  function navigate(next: URLSearchParams, replace = false) {
    intent.current++;
    setPreview(null);
    setSingle(null);
    const text = next.toString();
    const url = `${window.location.pathname}${text ? `?${text}` : ""}`;
    if (replace) window.history.replaceState(null, "", url);
    else window.history.pushState(null, "", url);
    setQuery(window.location.search);
  }
  useEffect(() => {
    const pop = () => setQuery(window.location.search);
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setLoadError("");
    const request = new URLSearchParams(apiQuery);
    if (snapshot.current?.scope === scope && lastReload.current === reload) {
      request.set("snapshot", snapshot.current.id);
      request.set("expected_epoch", snapshot.current.epoch);
    } else {
      snapshot.current = null;
      setSelected([]);
      setPreview(null);
    }
    lastReload.current = reload;
    void getJson<ConnectionsOverview>(
      `api/v1/connections${request.toString() ? `?${request}` : ""}`,
      controller.signal,
    )
      .then((result) => {
        if (controller.signal.aborted) return;
        if (lastEpoch.current && lastEpoch.current !== result.epoch) {
          intent.current++;
          setNotice("服务观测周期已变化，已重新建立连接快照。");
          setSelected([]);
          setPreview(null);
          setSingle(null);
        }
        setRates(sampleRates(previous.current, result.statistics));
        previous.current = result.statistics;
        lastEpoch.current = result.epoch;
        snapshot.current = { id: result.snapshot, epoch: result.epoch, scope };
        setValue(result);
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted) return;
        if (
          request.has("snapshot") &&
          cause instanceof ApiRequestError &&
          recoverable.has(cause.code ?? "")
        ) {
          snapshot.current = null;
          previous.current = null;
          setSelected([]);
          setPreview(null);
          setSingle(null);
          setNotice("原快照已失效，已重置分页与选择；断开范围需要重新预览。");
          const next = new URLSearchParams(window.location.search);
          if (next.has("offset")) {
            next.delete("offset");
            navigate(next, true);
          } else setReload((n) => n + 1);
          return;
        }
        setValue(null);
        setRates(null);
        previous.current = null;
        if (
          cause instanceof ApiRequestError &&
          [400, 404, 422].includes(cause.status ?? 0)
        )
          setAutomatic(false);
        setLoadError(message(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [apiQuery, scope, reload]);
  useEffect(() => {
    if (!automatic || loading || busy || preview || single) return;
    const timer = window.setTimeout(() => {
      if (!document.hidden) setReload((n) => n + 1);
    }, 2000);
    const visible = () => {
      if (!document.hidden) setReload((n) => n + 1);
    };
    document.addEventListener("visibilitychange", visible);
    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", visible);
    };
  }, [automatic, loading, busy, preview, single, reload]);
  useEffect(() => {
    if (!detailId) {
      setDetail(null);
      setDetailError("");
      return;
    }
    const controller = new AbortController();
    setDetailError("");
    void getJson<ConnectionInfo>(
      `api/v1/connections/${encodeURIComponent(detailId)}`,
      controller.signal,
    )
      .then((result) => {
        if (!controller.signal.aborted) setDetail(result);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted) {
          setDetail(null);
          setDetailError(message(cause));
        }
      });
    return () => controller.abort();
  }, [detailId, value?.snapshot]);
  function filters(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;
    const fields = new FormData(event.currentTarget);
    const next = new URLSearchParams();
    for (const key of filterKeys) {
      if (["offset", "id"].includes(key)) continue;
      const v = String(fields.get(key) ?? "").trim();
      if (v) next.set(key, v);
    }
    snapshot.current = null;
    setSelected([]);
    setPreview(null);
    navigate(next);
    setReload((n) => n + 1);
  }
  function page(offset: number) {
    const next = new URLSearchParams(query);
    next.set("offset", String(offset));
    navigate(next);
  }
  function show(id: string | null) {
    const next = new URLSearchParams(query);
    if (id) next.set("detail", id);
    else {
      next.delete("detail");
      next.delete("id");
    }
    navigate(next);
  }
  function select(id: string, checked: boolean) {
    intent.current++;
    setSingle(null);
    setAutomatic(false);
    setSelected((ids) =>
      checked
        ? ids.length < 500 && !ids.includes(id)
          ? [...ids, id]
          : ids
        : ids.filter((i) => i !== id),
    );
    setPreview(null);
  }
  async function prepare() {
    if (!value) return;
    setBusy(true);
    setError("");
    setNotice("");
    setAutomatic(false);
    setSingle(null);
    setPreview(null);
    const generation = intent.current;
    try {
      const result = await postJson<DisconnectPreview>(
        "api/v1/connections/disconnect/preview",
        {
          snapshot: value.snapshot,
          expected_epoch: value.epoch,
          ids: selected,
        },
      );
      if (generation === intent.current) setPreview(result);
    } catch (cause) {
      if (generation !== intent.current) return;
      setError(message(cause));
      if (
        cause instanceof ApiRequestError &&
        recoverable.has(cause.code ?? "")
      ) {
        snapshot.current = null;
        setSelected([]);
        setReload((n) => n + 1);
      }
    } finally {
      setBusy(false);
    }
  }
  async function disconnect() {
    if (!single && !preview) return;
    setBusy(true);
    setError("");
    try {
      const result = single
        ? await postJson<DisconnectResult>("api/v1/connections/disconnect", {
            id: single.id,
            expected_epoch: single.epoch,
          })
        : await postJson<DisconnectResult>(
            "api/v1/connections/disconnect/confirm",
            { token: preview!.token, expected_epoch: preview!.epoch },
          );
      setNotice(
        `断开请求已受理：${result.requested} 条已请求取消，${result.already_ended} 条已结束。连接实际退出后计数归零，不迁移或重放数据。`,
      );
      setPreview(null);
      setSingle(null);
      setSelected([]);
      setReload((n) => n + 1);
    } catch (cause) {
      setError(message(cause));
      if (
        cause instanceof ApiRequestError &&
        recoverable.has(cause.code ?? "")
      ) {
        setPreview(null);
        setSingle(null);
        setSelected([]);
        snapshot.current = null;
        setReload((n) => n + 1);
      }
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="space-y-3">
      {value && (
        <ConnectionStats value={value.statistics} rates={rates} compact />
      )}
      <section className="panel space-y-2.5 p-3">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <h2 className="flex items-center gap-2 font-semibold">
              <Activity size={19} />
              业务连接与隧道
            </h2>
            <p className="muted mt-2 text-xs leading-6">
              按目标、客户端和状态查找 HTTP 业务交换或隧道。
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn"
              disabled={loading || busy}
              onClick={() => setReload((n) => n + 1)}
            >
              <RefreshCw size={15} />
              刷新连接
            </button>
            <button
              className="btn"
              disabled={busy}
              onClick={() => {
                setAutomatic(!automatic);
                if (!automatic) {
                  setSelected([]);
                  setReload((n) => n + 1);
                }
              }}
            >
              {automatic ? <Pause size={15} /> : <Play size={15} />}{" "}
              {automatic ? "暂停自动刷新" : "开始自动刷新"}
            </button>
          </div>
        </div>
        <form
          key={`${apiQuery}:${value?.inbounds.join(",") ?? ""}:${value?.groups.join(",") ?? ""}:${value?.providers.join(",") ?? ""}`}
          onSubmit={filters}
          className="space-y-2"
          aria-label="连接筛选"
        >
          <div className="filter-grid">
            <label className="form-field">
              目标地址或已使用域名
              <input
                className="field"
                name="destination"
                defaultValue={params.get("destination") ?? ""}
              />
            </label>
            <label className="form-field">
              客户端 IP / CIDR
              <input
                className="field"
                name="source"
                defaultValue={params.get("source") ?? ""}
              />
            </label>
            <label className="form-field">
              连接状态
              <select
                className="field"
                name="state"
                defaultValue={params.get("state") ?? "active"}
              >
                <option value="active">所有活动记录</option>
                <option value="all">活动及最近终态</option>
                <option value="ended">最近终态</option>
                {Object.entries(connectionStates).map(([key, label]) => (
                  <option value={key} key={key}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <div className="filter-actions">
              <button className="btn" disabled={busy}>
                <Filter size={15} />
                应用筛选
              </button>
              <button
                className="btn"
                type="button"
                disabled={busy}
                onClick={() => {
                  snapshot.current = null;
                  setSelected([]);
                  navigate(new URLSearchParams());
                  setReload((n) => n + 1);
                }}
              >
                重置筛选
              </button>
            </div>
          </div>
          <AdvancedFilters
            active={
              params.has("source_port") ||
              params.has("destination_port") ||
              params.has("inbound") ||
              params.has("protocol") ||
              params.has("transport") ||
              params.has("origin") ||
              params.has("node") ||
              params.has("group") ||
              params.has("provider") ||
              params.has("rule") ||
              params.has("error") ||
              params.has("created_after_unix_ms") ||
              (params.get("sort") ?? "newest") !== "newest" ||
              (params.get("limit") ?? "50") !== "50"
            }
          >
            <label className="form-field">
              客户端端口
              <input
                className="field"
                name="source_port"
                type="number"
                min="1"
                max="65535"
                defaultValue={params.get("source_port") ?? ""}
              />
            </label>
            <label className="form-field">
              目标端口
              <input
                className="field"
                name="destination_port"
                type="number"
                min="1"
                max="65535"
                defaultValue={params.get("destination_port") ?? ""}
              />
            </label>
            <label className="form-field">
              入口
              <input
                className="field"
                name="inbound"
                list="connection-inbounds"
                defaultValue={params.get("inbound") ?? ""}
              />
              <datalist id="connection-inbounds">
                {value?.inbounds.map((id) => (
                  <option key={id}>{id}</option>
                ))}
              </datalist>
            </label>
            <label className="form-field">
              入口协议
              <select
                className="field"
                name="protocol"
                defaultValue={params.get("protocol") ?? ""}
              >
                <option value="">全部入口协议</option>
                {["http", "https", "socks5", "transparent"].map((p) => (
                  <option key={p}>{p}</option>
                ))}
              </select>
            </label>
            <label className="form-field">
              传输
              <select
                className="field"
                name="transport"
                defaultValue={params.get("transport") ?? ""}
              >
                <option value="">全部传输</option>
                <option value="tcp">TCP</option>
                <option value="udp">UDP</option>
              </select>
            </label>
            <label className="form-field">
              来源类型
              <select
                className="field"
                name="origin"
                defaultValue={params.get("origin") ?? ""}
              >
                <option value="">全部来源类型</option>
                {Object.entries(origins).map(([key, label]) => (
                  <option value={key} key={key}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
            <label className="form-field">
              节点 ID（直连填 @direct）
              <input
                className="field"
                name="node"
                defaultValue={params.get("node") ?? ""}
              />
            </label>
            <label className="form-field">
              分组（未分组填 @none）
              <input
                className="field"
                name="group"
                list="connection-groups"
                defaultValue={params.get("group") ?? ""}
              />
              <datalist id="connection-groups">
                {value?.groups.map((id) => (
                  <option key={id}>{id}</option>
                ))}
              </datalist>
            </label>
            <label className="form-field">
              供应商（静态填 @static）
              <input
                className="field"
                name="provider"
                list="connection-providers"
                defaultValue={params.get("provider") ?? ""}
              />
              <datalist id="connection-providers">
                {value?.providers.map((id) => (
                  <option key={id}>{id}</option>
                ))}
              </datalist>
            </label>
            <label className="form-field">
              规则 ID（默认填 @default）
              <input
                className="field"
                name="rule"
                defaultValue={params.get("rule") ?? ""}
              />
            </label>
            <label className="form-field">
              错误码
              <input
                className="field"
                name="error"
                defaultValue={params.get("error") ?? ""}
              />
            </label>
            <FilterStartTime
              name="created_after_unix_ms"
              value={params.get("created_after_unix_ms") ?? ""}
            />
            <label className="form-field">
              排序
              <select
                className="field"
                name="sort"
                defaultValue={params.get("sort") ?? "newest"}
              >
                <option value="newest">最新开始</option>
                <option value="oldest">最早开始</option>
                <option value="bytes_up">上传量</option>
                <option value="bytes_down">下载量</option>
                <option value="elapsed">持续时间</option>
              </select>
            </label>
            <label className="form-field">
              每页
              <select
                className="field"
                name="limit"
                defaultValue={params.get("limit") ?? "50"}
              >
                {pageSizes.map((n) => (
                  <option key={n}>{n}</option>
                ))}
              </select>
            </label>
          </AdvancedFilters>
        </form>
        {value && (
          <details className="muted text-xs">
            <summary className="cursor-pointer">
              {automatic ? "每 2 秒刷新" : "自动刷新已暂停"} · 快照{" "}
              {date(value.snapshot_created_at_unix_ms)} · r
              {value.running_revision}
            </summary>
            <p className="mt-2 leading-5">
              快照最多保留 30 秒，实例变化或快照失效后重新读取。最近终态最多保留
              {value.statistics.recent_record_limit} 条、最长 5 分钟。
            </p>
          </details>
        )}
        {value && value.statistics.active_untracked > 0 && (
          <p role="alert" className="text-sm text-amber-700">
            当前有 {value.statistics.active_untracked}{" "}
            条未完整观测的活动交换。当前列表和断开预览只涵盖已记录的具体
            ID，不能视为全部连接。
          </p>
        )}
      </section>
      {loadError && (
        <p role="alert" className="panel break-words p-4 text-sm text-red-600">
          连接读取失败：{loadError}
        </p>
      )}
      {error && (
        <p role="alert" className="panel break-words p-4 text-sm text-red-600">
          {error}
        </p>
      )}
      {notice && (
        <p
          role="status"
          className="panel break-words p-4 text-sm text-emerald-700"
        >
          {notice}
        </p>
      )}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="muted text-xs">
          匹配 {value?.filtered_total ?? "—"} 条 · 已选择 {selected.length}{" "}
          条。勾选会暂停自动刷新以保留选择范围。
        </p>
        <div className="flex flex-wrap gap-2">
          <button
            className="btn"
            disabled={busy || !selected.length}
            onClick={() => setSelected([])}
          >
            清除选择
          </button>
          <button
            className="btn"
            disabled={
              busy ||
              loading ||
              !value ||
              !!loadError ||
              (!selected.length && value.filtered_total === 0)
            }
            onClick={() => void prepare()}
          >
            <Unplug size={15} />
            {selected.length ? "预览断开所选连接" : "预览断开当前筛选"}
          </button>
        </div>
      </div>
      {(single || preview) && (
        <section
          className="panel space-y-4 border-amber-300 p-5 dark:border-amber-900"
          aria-label="断开确认"
        >
          <h2 className="font-semibold">确认断开具体连接</h2>
          <p className="text-sm leading-7">
            {single
              ? "本次只作用于下面的连接 ID。"
              : "本次仅作用于预览列出的 ID，之后新出现的连接不会加入范围。"}
            断开会中止业务，HTTP 交换还可能关闭复用的入口 keep-alive
            套接字；不会换节点、迁移或重放。
            {preview &&
              `预览时间 ${date(preview.created_at_unix_ms)}，有效期最多 30 秒。`}
          </p>
          <div className="max-h-64 space-y-2 overflow-auto">
            {single ? (
              <div className="break-all rounded-lg bg-slate-50 p-3 text-xs dark:bg-slate-800">
                {single.id}
                <br />
                {endpointLabel(
                  single.context.destination_host,
                  single.context.destination_port,
                )}{" "}
                · {single.choice?.node_id ?? single.routing.action.kind}
              </div>
            ) : (
              preview?.targets.map((target) => (
                <div
                  key={target.id}
                  className="break-all rounded-lg bg-slate-50 p-3 text-xs dark:bg-slate-800"
                >
                  {target.id}
                  <br />
                  {target.source_ip ?? "未知来源"}:{target.source_port ?? "—"} →{" "}
                  {endpointLabel(
                    target.destination_host,
                    target.destination_port,
                  )}{" "}
                  · {target.node_id ?? "无代理节点"} · r
                  {target.configuration_revision}
                </div>
              ))
            )}
          </div>
          {preview?.active_untracked_in_instance ? (
            <p className="text-xs text-amber-700">
              实例另有 {preview.active_untracked_in_instance}{" "}
              条未完整观测连接，不在本次范围内。
            </p>
          ) : null}
          <div className="flex flex-wrap gap-2">
            <button
              className="btn"
              disabled={busy || (!single && !preview?.targets.length)}
              onClick={() => void disconnect()}
            >
              确认断开 {single ? 1 : (preview?.targets.length ?? 0)} 条连接
            </button>
            <button
              className="btn"
              disabled={busy}
              onClick={() => {
                setPreview(null);
                setSingle(null);
              }}
            >
              取消断开
            </button>
          </div>
        </section>
      )}
      {detailId && (
        <section
          ref={detailRegion}
          tabIndex={-1}
          className="panel"
          aria-label="连接路由详情"
        >
          <div className="flex items-center justify-between gap-3 border-b border-slate-100 p-5 dark:border-slate-800">
            <h2 className="font-semibold">连接路由详情</h2>
            <button className="btn" onClick={() => show(null)}>
              <X size={14} />
              关闭详情
            </button>
          </div>
          {detailError ? (
            <p className="break-words p-5 text-sm text-amber-700">
              {detailError}
            </p>
          ) : detail && detail.id === detailId ? (
            <>
              <Details
                rows={[
                  ["连接 ID", <code className="text-xs">{detail.id}</code>],
                  ["状态 / 阶段", `${stateLabel(detail)} / ${detail.phase}`],
                  [
                    "创建 / 结束",
                    `${date(detail.created_at_unix_ms)} / ${date(detail.ended_at_unix_ms)}`,
                  ],
                  [
                    "来源",
                    `${detail.context.source_ip ?? "未知"}:${detail.context.source_port ?? "未知端口"} · ${origins[detail.context.origin] ?? detail.context.origin}`,
                  ],
                  [
                    "入口",
                    `${detail.context.inbound ?? "未知"} / ${detail.context.inbound_protocol ?? "未知协议"}`,
                  ],
                  [
                    "目标",
                    endpointLabel(
                      detail.context.destination_host,
                      detail.context.destination_port,
                    ),
                  ],
                  [
                    "命中规则",
                    <a
                      className="text-blue-600"
                      href={`./rules.html?revision=${detail.routing.configuration_revision}`}
                    >
                      {detail.routing.rule_id ?? "默认动作"} · r
                      {detail.routing.configuration_revision}
                    </a>,
                  ],
                  [
                    "实际动作",
                    `${detail.routing.action.kind}${detail.routing.action.id ? ` / ${detail.routing.action.id}` : ""}`,
                  ],
                  [
                    "选点",
                    detail.selection
                      ? `${detail.selection.selected ?? "未选中"} · ${strategyLabels[detail.selection.strategy] ?? detail.selection.strategy} · ${candidateReasons[detail.selection.reason] ?? detail.selection.reason}`
                      : "此动作不使用代理选点",
                  ],
                  [
                    "节点",
                    detail.choice ? (
                      <a
                        className="text-blue-600"
                        href={`./node-detail.html?id=${encodeURIComponent(detail.choice.node_id)}`}
                      >
                        {detail.choice.node_id} ·{" "}
                        {detail.choice.upstream_protocol}://
                        {endpointLabel(
                          detail.choice.upstream_host,
                          detail.choice.upstream_port,
                        )}
                      </a>
                    ) : (
                      "无代理节点"
                    ),
                  ],
                  [
                    "来源 / 组",
                    `${detail.choice?.provider_id ?? "无供应商"} / ${detail.selection?.group ?? "未分组"}`,
                  ],
                  [
                    "节点寿命",
                    detail.choice
                      ? `${detail.choice.expires_at_unix === null ? "未设到期" : date(detail.choice.expires_at_unix * 1000)} · 选点时约 ${detail.choice.remaining_secs_at_admission ?? "未知"} 秒，最低 ${detail.choice.minimum_remaining_secs} 秒`
                      : "无节点租约",
                  ],
                  [
                    "域名上下文",
                    `${detail.routing.domain ?? "未知"} · ${domainSources[detail.routing.domain_source] ?? detail.routing.domain_source}`,
                  ],
                  [
                    "进程",
                    detail.context.process
                      ? `${detail.context.process.command} / PID ${detail.context.process.pid} / cgroup ${detail.context.process.cgroup_id}`
                      : "未知；显式代理不会推断远端客户端的进程",
                  ],
                  [
                    "出口业务读写",
                    `上传 ${formatBytes(detail.bytes_up)} / 下载 ${formatBytes(detail.bytes_down)}`,
                  ],
                  [
                    "HTTP 状态 / 异常",
                    `${detail.http_status ?? "不可见或未收到"} / ${detail.error_code ? (connectionErrors[detail.error_code] ?? detail.error_code) : "无已记录异常"}`,
                  ],
                  ["关闭原因", detail.close_reason ?? "尚未释放"],
                ]}
              />
              <div className="p-5">
                <button
                  className="btn"
                  disabled={busy || !active(detail)}
                  onClick={() => {
                    setSingle(detail);
                    setPreview(null);
                    setAutomatic(false);
                  }}
                >
                  断开此连接
                </button>
              </div>
            </>
          ) : (
            <p className="muted p-5 text-sm">正在读取连接详情…</p>
          )}
        </section>
      )}
      {loading && <p className="muted text-sm">正在读取连接快照…</p>}
      <div
        className={value?.records.length ? "panel" : undefined}
        aria-label="连接列表"
      >
        {value?.records.map((record) => (
          <article
            className="management-record"
            key={record.id}
            aria-label={`连接 ${record.id}`}
          >
            <div className="min-w-0">
              <div className="flex flex-wrap items-start gap-2">
                <label className="flex min-w-0 items-start gap-2.5">
                  <input
                    type="checkbox"
                    className="mt-1 shrink-0"
                    aria-label={`选择连接 ${record.id}`}
                    disabled={!active(record) || busy || loading}
                    checked={selected.includes(record.id)}
                    onChange={(e) => select(record.id, e.target.checked)}
                  />
                  <span className="min-w-0 [overflow-wrap:anywhere]">
                    <span className="block text-sm font-semibold">
                      {endpointLabel(
                        record.context.destination_host,
                        record.context.destination_port,
                      )}
                    </span>
                    <span className="muted block font-mono text-xs">
                      {record.id}
                    </span>
                  </span>
                </label>
                <Badge active={active(record) && record.state === "open"}>
                  {stateLabel(record)}
                </Badge>
              </div>
              <p className="muted mt-1 break-words text-xs [overflow-wrap:anywhere]">
                {record.context.source_ip ?? "未知来源"}:
                {record.context.source_port ?? "—"} →{" "}
                {record.context.inbound ?? "未知入口"} ·{" "}
                {record.context.inbound_protocol ?? record.context.transport} ·{" "}
                {origins[record.context.origin]}
              </p>
            </div>
            <div className="grid min-w-0 grid-cols-2 gap-x-3 gap-y-0.5 text-xs tabular-nums [overflow-wrap:anywhere]">
              <p className="break-all">
                出口：{record.choice?.node_id ?? record.routing.action.kind}
              </p>
              <p>
                规则：{record.routing.rule_id ?? "默认"} · r
                {record.routing.configuration_revision}
              </p>
              <p>
                上传 / 下载：{formatBytes(record.bytes_up)} /{" "}
                {formatBytes(record.bytes_down)}
              </p>
              <p>快照时持续：{(record.elapsed_ms / 1000).toFixed(1)} s</p>
              {record.error_code && (
                <p className="col-span-2 break-words text-amber-700">
                  {connectionErrors[record.error_code] ?? record.error_code}
                </p>
              )}
            </div>
            <div className="flex flex-wrap gap-2">
              <button className="btn text-xs" onClick={() => show(record.id)}>
                <Search size={14} />
                路由详情
              </button>
              <button
                className="btn text-xs"
                disabled={busy || !active(record)}
                onClick={() => {
                  setSingle(record);
                  setPreview(null);
                  setAutomatic(false);
                }}
              >
                断开
              </button>
            </div>
          </article>
        ))}
      </div>
      {value && !loading && value.records.length === 0 && (
        <p className="panel p-8 text-center text-sm">
          当前快照没有符合条件的完整连接记录。可调整筛选、查看最近终态或刷新。
        </p>
      )}
      {value && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="muted text-xs">
            匹配 {value.filtered_total} · 当前偏移 {value.offset}
          </p>
          <div className="flex gap-2">
            <button
              className="btn"
              disabled={loading || busy || value.offset === 0}
              onClick={() =>
                page(
                  Math.max(0, value.offset - Number(params.get("limit") ?? 50)),
                )
              }
            >
              上一页
            </button>
            <button
              className="btn"
              disabled={loading || busy || value.next_offset === null}
              onClick={() =>
                value.next_offset !== null && page(value.next_offset)
              }
            >
              下一页
            </button>
          </div>
        </div>
      )}
      <ObservationPanel mode="connections" />
    </div>
  );
}
