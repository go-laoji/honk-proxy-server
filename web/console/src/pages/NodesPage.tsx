import { useEffect, useRef, useState, type FormEvent } from "react";
import { Globe2, RefreshCw, Upload, Filter, ArrowLeft } from "lucide-react";
import { Badge, Details } from "../components/ui";
import { NodeStats, SourceStats } from "../components/NodeSummary";
import { getJson, postJson } from "../platform";
import {
  nodeAddress,
  nodeReasons,
  nodeStates,
  type NodesOverview,
  type NodeView,
  type ImportRequest,
  type ImportPreview,
} from "../nodes";

const message = (cause: unknown) =>
  cause instanceof Error ? cause.message : String(cause);
const date = (value: number | null) =>
  value === null ? "未提供" : new Date(value * 1000).toLocaleString("zh-CN");
const filters = [
  "search",
  "provider",
  "group",
  "protocol",
  "capability",
  "state",
  "remaining_max_secs",
  "sort",
  "limit",
];

export default function NodesPage() {
  const [query, setQuery] = useState(window.location.search);
  const [reload, setReload] = useState(0);
  const [value, setValue] = useState<NodesOverview | null>(null);
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(true);
  const [editing, setEditing] = useState(false);
  const [document, setDocument] = useState("");
  const [format, setFormat] = useState("lines");
  const [protocol, setProtocol] = useState("http");
  const [selectedRows, setSelectedRows] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [reviewed, setReviewed] = useState<ImportRequest | null>(null);
  const generation = useRef(0);
  const params = new URLSearchParams(query);
  const pageSizes = Array.from(
    new Set([20, 50, 100, 200, Number(params.get("limit") ?? 50)]),
  )
    .filter((n) => Number.isInteger(n) && n >= 1 && n <= 200)
    .sort((a, b) => a - b);
  const selectedId = params.get("id");
  useEffect(() => {
    const pop = () => setQuery(window.location.search);
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    const request = ++generation.current;
    setLoading(true);
    setLoadError("");
    void getJson<NodesOverview>(`api/v1/nodes${query}`, controller.signal)
      .then((result) => {
        if (!controller.signal.aborted && generation.current === request)
          setValue(result);
      })
      .catch((cause: unknown) => {
        if (!controller.signal.aborted && generation.current === request) {
          setLoadError(message(cause));
          setValue(null);
        }
      })
      .finally(() => {
        if (!controller.signal.aborted && generation.current === request)
          setLoading(false);
      });
    return () => controller.abort();
  }, [query, reload]);
  useEffect(() => {
    if (!document) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [document]);
  function navigate(next: URLSearchParams) {
    const search = next.toString();
    if (window.location.pathname.endsWith("/node-detail.html")) {
      window.location.assign(`./nodes.html${search ? `?${search}` : ""}`);
      return;
    }
    const url = `${window.location.pathname}${search ? `?${search}` : ""}`;
    window.history.pushState(null, "", url);
    setQuery(window.location.search);
  }
  function filter(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const next = new URLSearchParams();
    for (const key of filters) {
      const v = String(form.get(key) ?? "").trim();
      if (v) next.set(key, v);
    }
    navigate(next);
  }
  function page(offset: number) {
    const next = new URLSearchParams(query);
    next.set("offset", String(offset));
    navigate(next);
  }
  function changed() {
    setPreview(null);
    setReviewed(null);
  }
  async function action(node: NodeView, kind: string) {
    if (!value) return;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const result = await postJson<{ pool_refreshed: boolean }>(
        "api/v1/nodes/action",
        {
          id: node.id,
          action: kind,
          expected_revision: value.running_revision,
          expected_control_version: node.operation.version,
          lease_id: node.lease_id,
          batch_id: node.batch_id,
          provider_id: node.provider_id,
        },
      );
      setNotice(
        `${node.id}：${kind === "enable" ? "已解除人工限制，仍需通过配置、健康与寿命准入" : kind === "delete" ? "已软删除，仅抑制此节点，可恢复" : kind === "drain" ? "已停止新连接，现有连接自然排空" : "已禁用新连接"}`,
      );
      if (!result.pool_refreshed)
        setError("节点操作已生效，当前池尚未完成重算；请刷新后重试此操作。");
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
      setReload((n) => n + 1);
    }
  }
  async function importPreview() {
    if (!value) return;
    setBusy(true);
    setError("");
    setNotice("");
    setPreview(null);
    setReviewed(null);
    try {
      const selected = selectedRows.trim()
        ? selectedRows.split(",").map((s) => Number(s.trim()))
        : [];
      if (selected.some((n) => !Number.isInteger(n) || n < 1))
        throw new Error("选择行请填写逗号分隔的正整数");
      const input: ImportRequest = {
        document,
        format,
        default_protocol: protocol,
        expected_revision: value.running_revision,
        preview_checksum: null,
        selected_rows: selected,
      };
      const result = await postJson<ImportPreview>(
        "api/v1/nodes/import/preview",
        input,
      );
      setPreview(result);
      setReviewed({ ...input, preview_checksum: result.checksum });
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  }
  async function applyImport() {
    if (!preview?.applicable || !reviewed) return;
    setBusy(true);
    setError("");
    try {
      await postJson("api/v1/nodes/import/apply", reviewed);
      setDocument("");
      setPreview(null);
      setReviewed(null);
      setEditing(false);
      setSelectedRows("");
      setNotice(
        `已导入并激活 ${preview.selected_count} 个静态节点；健康检查和业务准入独立判断。`,
      );
      setReload((n) => n + 1);
    } catch (cause) {
      setError(message(cause));
      setReload((n) => n + 1);
    } finally {
      setBusy(false);
    }
  }
  const selected = value?.nodes.find((n) => n.id === selectedId);
  return (
    <div className="space-y-6">
      {value && <NodeStats value={value.summary} />}
      {value && value.sources.length > 0 && (
        <SourceStats sources={value.sources} />
      )}
      <section className="panel p-5">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h2 className="flex items-center gap-2 font-semibold">
              <Globe2 size={19} />
              节点库存
            </h2>
            <p className="muted mt-2 text-xs leading-6">
              配置、租约、健康与人工限制分别判断。禁用、排空和软删除均停止新拨号，已有连接继续排空。
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn"
              disabled={busy || loading}
              onClick={() => setReload((n) => n + 1)}
            >
              <RefreshCw size={15} />
              刷新节点
            </button>
            <button
              className="btn"
              disabled={busy}
              onClick={() => setEditing(!editing)}
            >
              <Upload size={15} />
              添加 / 导入
            </button>
          </div>
        </div>
        <form
          key={`${query}:${value?.providers.join(",") ?? ""}:${value?.groups.join(",") ?? ""}`}
          onSubmit={filter}
          className="mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-4"
          aria-label="节点筛选"
        >
          <label className="text-xs">
            名称或端点
            <input
              className="field mt-1"
              name="search"
              defaultValue={params.get("search") ?? ""}
            />
          </label>
          <label className="text-xs">
            来源
            <select
              className="field mt-1"
              name="provider"
              defaultValue={params.get("provider") ?? ""}
            >
              <option value="">全部来源</option>
              <option value="@static">静态节点</option>
              {value?.providers.map((id) => (
                <option key={id}>{id}</option>
              ))}
            </select>
          </label>
          <label className="text-xs">
            分组
            <select
              className="field mt-1"
              name="group"
              defaultValue={params.get("group") ?? ""}
            >
              <option value="">全部分组</option>
              {value?.groups.map((id) => (
                <option key={id}>{id}</option>
              ))}
            </select>
          </label>
          <label className="text-xs">
            协议
            <select
              className="field mt-1"
              name="protocol"
              defaultValue={params.get("protocol") ?? ""}
            >
              <option value="">全部协议</option>
              {["http", "https", "socks5"].map((p) => (
                <option key={p}>{p}</option>
              ))}
            </select>
          </label>
          <label className="text-xs">
            能力
            <select
              className="field mt-1"
              name="capability"
              defaultValue={params.get("capability") ?? ""}
            >
              <option value="">全部能力</option>
              <option value="tcp">TCP</option>
              <option value="udp">UDP</option>
              <option value="ipv4">IPv4 健康已证实</option>
              <option value="ipv6">IPv6 健康已证实</option>
            </select>
          </label>
          <label className="text-xs">
            状态
            <select
              className="field mt-1"
              name="state"
              defaultValue={params.get("state") ?? ""}
            >
              <option value="">全部状态</option>
              {Object.entries({
                eligible: "可准入",
                blocked: "不可准入",
                near_expiry: "60 秒内到期",
                expired: "已到期",
                disabled: "已禁用",
                draining: "排空中",
                deleted: "软删除",
                retired: "已退休 / 来源不可用",
                healthy: "健康通过",
                unhealthy: "检测失败",
                unknown: "健康未知",
              }).map(([v, label]) => (
                <option value={v} key={v}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label className="text-xs">
            剩余寿命不超过（秒）
            <input
              className="field mt-1"
              name="remaining_max_secs"
              type="number"
              min="0"
              defaultValue={params.get("remaining_max_secs") ?? ""}
            />
          </label>
          <label className="text-xs">
            排序
            <select
              className="field mt-1"
              name="sort"
              defaultValue={params.get("sort") ?? "id"}
            >
              <option value="id">节点 ID</option>
              <option value="expiry_asc">最早到期</option>
              <option value="expiry_desc">最晚到期</option>
            </select>
          </label>
          <label className="text-xs">
            每页
            <select
              className="field mt-1"
              name="limit"
              defaultValue={params.get("limit") ?? "50"}
            >
              {pageSizes.map((n) => (
                <option key={n}>{n}</option>
              ))}
            </select>
          </label>
          <div className="flex items-end gap-2">
            <button className="btn" disabled={busy}>
              <Filter size={15} />
              应用筛选
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => navigate(new URLSearchParams())}
            >
              重置
            </button>
          </div>
        </form>
        {value && (
          <p className="muted mt-4 text-xs">
            运行配置 r{value.running_revision} · {date(value.evaluated_at_unix)}{" "}
            · 匹配 {value.filtered_total}{" "}
            个节点。分页反映实时库存，写操作校验所见版本。
          </p>
        )}
        {value?.inventory_truncated && (
          <p role="alert" className="mt-3 text-sm text-amber-700">
            库存查询达到 200000 条租约上限，当前摘要为受限样本。
          </p>
        )}
      </section>
      {(error || loadError) && (
        <p role="alert" className="panel break-words p-4 text-sm text-red-600">
          {error || loadError}
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
      {editing && (
        <section className="panel space-y-4 p-5" aria-label="节点导入">
          <h2 className="font-semibold">添加节点与批量导入</h2>
          <p className="muted text-xs leading-6">
            每行一个 HTTP / HTTPS / SOCKS5 地址或 host:port；支持 URL
            中的上游凭据。JSON 使用静态 UpstreamConfig 数组。最多 1000 条、1
            MiB；预览不发起连接或购买。输入可能包含凭据，离开页面前请完成或清空。
          </p>
          <div className="grid gap-3 sm:grid-cols-3">
            <label className="text-xs">
              导入格式
              <select
                className="field mt-1"
                value={format}
                onChange={(e) => {
                  setFormat(e.target.value);
                  changed();
                }}
              >
                <option value="lines">逐行地址</option>
                <option value="json">JSON 数组</option>
              </select>
            </label>
            <label className="text-xs">
              无协议地址默认使用
              <select
                className="field mt-1"
                value={protocol}
                onChange={(e) => {
                  setProtocol(e.target.value);
                  changed();
                }}
              >
                {["http", "https", "socks5"].map((p) => (
                  <option key={p}>{p}</option>
                ))}
              </select>
            </label>
            <label className="text-xs">
              仅应用行号（可选，逗号分隔）
              <input
                className="field mt-1"
                value={selectedRows}
                onChange={(e) => {
                  setSelectedRows(e.target.value);
                  changed();
                }}
              />
            </label>
          </div>
          <label className="block text-xs">
            节点导入内容
            <textarea
              className="field mt-2 min-h-40 font-mono text-xs"
              value={document}
              onChange={(e) => {
                setDocument(e.target.value);
                changed();
              }}
              spellCheck={false}
            />
          </label>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn"
              disabled={busy || !document.trim() || !value}
              onClick={() => void importPreview()}
            >
              预览并校验
            </button>
            <button
              className="btn"
              disabled={busy}
              onClick={() => {
                setDocument("");
                changed();
              }}
            >
              清空内容
            </button>
          </div>
          {preview && (
            <div aria-label="导入预览" className="space-y-3">
              <p className="text-sm">
                有效 {preview.valid_count} · 无效 {preview.invalid_count} · 选择{" "}
                {preview.selected_count}。
                {preview.applicable
                  ? "所选行可提交；激活时仍校验引用、资源和版本。"
                  : "请修正无效行，或明确填写有效行号后重新预览。"}
              </p>
              <div className="max-h-80 overflow-auto rounded-lg border border-slate-200 dark:border-slate-700">
                {preview.rows.map((row) => (
                  <div
                    key={row.row}
                    className="break-all border-b border-slate-100 p-3 text-xs dark:border-slate-800"
                  >
                    第 {row.row} 行 ·{" "}
                    {row.valid
                      ? `${row.protocol}://${row.host}:${row.port} · ${row.id}`
                      : `不支持 / 无效：${row.error_code}`}{" "}
                    {row.has_credentials && "· 含上游凭据（已隐藏）"}
                  </div>
                ))}
              </div>
              <button
                className="btn"
                disabled={busy || !preview.applicable || !reviewed}
                onClick={() => void applyImport()}
              >
                确认导入并激活所选节点
              </button>
            </div>
          )}
        </section>
      )}
      {selectedId && (
        <section className="panel" aria-label="节点详情">
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-slate-100 p-5 dark:border-slate-800">
            <h2 className="break-all font-semibold">节点详情 · {selectedId}</h2>
            <a className="btn" href="./nodes.html">
              <ArrowLeft size={15} />
              返回列表
            </a>
          </div>
          {selected ? (
            <>
              <Details
                rows={[
                  [
                    "来源",
                    selected.provider_id ? (
                      <a
                        className="text-blue-600"
                        href={`./provider-detail.html?id=${encodeURIComponent(selected.provider_id)}`}
                      >
                        {selected.provider_id}
                      </a>
                    ) : (
                      "静态配置"
                    ),
                  ],
                  [
                    "租约 / 批次",
                    selected.lease_id === null
                      ? "无供应商租约"
                      : `#${selected.lease_id} / #${selected.batch_id}`,
                  ],
                  [
                    "接收 / 到期",
                    `${date(selected.received_at)} / ${date(selected.expires_at_unix)}`,
                  ],
                  [
                    "有效期依据",
                    selected.expiry_basis ??
                      "静态配置；未设置到期时间时不推断租约",
                  ],
                  [
                    "上游凭据",
                    selected.has_credentials ? "已配置（隐藏）" : "未配置",
                  ],
                  [
                    "分组",
                    selected.groups.length
                      ? selected.groups.map((id) => (
                          <a
                            className="mr-2 text-blue-600"
                            key={id}
                            href={`./groups.html?group=${encodeURIComponent(id)}`}
                          >
                            {id}
                          </a>
                        ))
                      : "没有匹配分组",
                  ],
                  ["标签", selected.tags.join("、") || "无"],
                  [
                    "业务拨号与连接",
                    <a
                      className="text-blue-600"
                      href={`./connections.html?node=${encodeURIComponent(selected.id)}`}
                    >
                      {selected.active_connections}，查看业务交换 / 隧道 →
                    </a>,
                  ],
                  [
                    "能力",
                    `TCP ${selected.tcp_supported ? "支持" : "不支持"} / UDP ${selected.udp_supported ? "支持" : "不支持"}；IPv4/IPv6 以实际健康证据为准`,
                  ],
                ]}
              />
              <div className="space-y-3 border-t border-slate-100 p-5 dark:border-slate-800">
                <a
                  href={`./health.html?node=${encodeURIComponent(selected.id)}`}
                  className="text-sm text-blue-600"
                >
                  健康检测与任务 →
                </a>
                {selected.health_results.length ? (
                  selected.health_results.map((r) => (
                    <p
                      key={r.profile_fingerprint}
                      className="break-words text-xs"
                    >
                      {r.transport.toUpperCase()} / {r.family} · {r.verdict} ·{" "}
                      {r.phase} · {date(r.checked_at)}
                      {r.error_code ? ` · ${r.error_code}` : ""}
                    </p>
                  ))
                ) : (
                  <p className="muted text-xs">
                    当前没有此节点的检测证据；不会把租约到期当作检测失败。
                  </p>
                )}
              </div>
            </>
          ) : (
            !loading && (
              <p className="muted p-5 text-sm">
                当前筛选下没有该节点，可返回列表或重置筛选。
              </p>
            )
          )}
        </section>
      )}
      {loading && <p className="muted text-sm">正在读取实际节点库存…</p>}
      <div className="grid gap-4 lg:grid-cols-2" aria-label="节点列表">
        {value?.nodes.map((node) => (
          <article
            className="panel min-w-0 space-y-4 p-5"
            key={node.id}
            aria-label={`节点 ${node.id}`}
          >
            <div className="flex flex-wrap items-start justify-between gap-2">
              <a
                className="break-all text-left font-semibold text-blue-600"
                href={`./node-detail.html?id=${encodeURIComponent(node.id)}`}
              >
                {node.id}
              </a>
              <Badge active={node.eligible}>
                {node.eligible ? "可准入" : "不可准入"}
              </Badge>
            </div>
            <p className="break-all font-mono text-xs">{nodeAddress(node)}</p>
            <div className="grid gap-2 text-xs sm:grid-cols-2">
              <p>租约：{nodeStates[node.lease_state] ?? node.lease_state}</p>
              <p>健康：{nodeStates[node.health_state] ?? node.health_state}</p>
              <p>人工状态：{nodeStates[node.operation.state]}</p>
              <p>
                剩余：
                {node.remaining_secs === null
                  ? "未设到期"
                  : `${node.remaining_secs} 秒`}
              </p>
              <p>
                来源：{node.provider_id ?? "静态节点"}
                {node.batch_id !== null ? ` / 批次 #${node.batch_id}` : ""}
              </p>
              <p>业务拨号 / 连接：{node.active_connections}</p>
            </div>
            {node.admission_reasons.length > 0 && (
              <ul className="space-y-1 text-xs text-amber-700 dark:text-amber-300">
                {node.admission_reasons.map((reason) => (
                  <li key={reason}>{nodeReasons[reason] ?? reason}</li>
                ))}
              </ul>
            )}
            <div className="flex flex-wrap gap-2">
              {[
                ["enable", "解除限制"],
                ["disable", "禁用"],
                ["drain", "排空"],
                ["delete", "软删除"],
              ].map(([kind, label]) => (
                <button
                  key={kind}
                  className="btn text-xs"
                  disabled={
                    busy ||
                    loading ||
                    (kind === "delete" && node.operation.state === "deleted")
                  }
                  onClick={() => void action(node, kind)}
                >
                  {label}
                </button>
              ))}
            </div>
          </article>
        ))}
      </div>
      {value && !loading && value.nodes.length === 0 && (
        <p className="panel p-8 text-center text-sm">
          没有符合筛选的节点；可调整条件或导入节点。
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
              disabled={loading || value.offset === 0}
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
              disabled={loading || value.next_offset === null}
              onClick={() =>
                value.next_offset !== null && page(value.next_offset)
              }
            >
              下一页
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
