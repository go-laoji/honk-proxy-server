import { ConfigFields, ConfigForm } from "../components/config/ConfigFields";
import {
  routingFields,
  ruleFields,
  simulationFields,
} from "../components/config/routingFields";
import { useEffect, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  Waypoints,
} from "lucide-react";
import { Badge } from "../components/ui";
import ObservationPanel from "../components/ObservationPanel";
import RuleImportPanel from "../components/RuleImportPanel";
import { getJson, postJson } from "../platform";
import { candidateReasons, type SelectionDecision } from "../selection";

type Action =
  | { kind: "default" | "direct" | "reject" }
  | { kind: "node" | "group"; id: string };
type Conditions = {
  domains?: string[];
  domain_suffixes?: string[];
  destination_cidrs?: string[];
  destination_ports?: { start: number; end: number }[];
  transports?: string[];
  inbound_protocols?: string[];
  inbounds?: string[];
  source_cidrs?: string[];
  process?: {
    pid?: number | null;
    command?: string | null;
    cgroup_id?: number | null;
  } | null;
};
type Rule = {
  id: string;
  enabled: boolean;
  conditions: Conditions;
  action: Action;
};
type Policy = {
  rules: Rule[];
  default_action: Action;
  use_dns_observations: boolean;
};
type Revision = { id: number; parent_id: number | null; created_at: number };
type Overview = {
  running_revision: number;
  desired_revision: number;
  viewed_revision: number;
  viewed_parent_revision: number | null;
  configuration: Policy;
  active_configuration: Policy;
  candidates: Revision[];
  changes: {
    path: string;
    before: unknown;
    after: unknown;
    apply_mode: string;
  }[];
  other_configuration_changes: boolean;
  capabilities: {
    local_process_available: boolean;
    local_process_reason: string;
    udp_data_plane_available: boolean;
    kernel_rule_revision: number | null;
    kernel_rules_ready: boolean;
  };
};
type Simulation = {
  decision: {
    configuration_revision: number;
    evaluated_at_unix: number;
    rule_id: string | null;
    action: Action;
    domain: string | null;
    domain_source: string;
    trace: { id: string; matched: boolean; reasons: string[] }[];
  };
  selection: SelectionDecision | null;
  action_supported: boolean;
  health_evaluated_at_unix: number | null;
  notes: string[];
};
const ruleReasons: Record<string, string> = {
  disabled: "规则已停用",
  domain_unknown: "目标域名未知",
  domain_mismatch: "域名不匹配",
  destination_ip_unknown: "没有字面目标 IP，不额外解析域名",
  destination_cidr_mismatch: "目的 CIDR 不匹配",
  destination_port_mismatch: "目的端口不匹配",
  transport_mismatch: "传输不匹配",
  inbound_protocol_unknown_or_mismatch: "入口协议未知或不匹配",
  inbound_unknown_or_mismatch: "入口未知或不匹配",
  source_ip_unknown: "源 IP 未提供",
  source_cidr_mismatch: "来源 CIDR 不匹配",
  verified_local_process_unavailable: "没有已验证的本机进程来源",
  process_mismatch: "本机进程不匹配",
};
function actionName(action: Action) {
  if (action.kind === "node") return `节点 ${action.id}`;
  if (action.kind === "group") return `分组 ${action.id}`;
  return { default: "入口 / 默认组选点", direct: "明确直连", reject: "拒绝" }[
    action.kind
  ];
}
function description(c: Conditions) {
  const parts = [
    c.domains?.length ? `域名 ${c.domains.join(", ")}` : "",
    c.domain_suffixes?.length ? `后缀 ${c.domain_suffixes.join(", ")}` : "",
    c.destination_cidrs?.length ? `目的 ${c.destination_cidrs.join(", ")}` : "",
    c.destination_ports?.length
      ? `端口 ${c.destination_ports.map((p) => (p.start === p.end ? p.start : `${p.start}-${p.end}`)).join(", ")}`
      : "",
    c.transports?.length ? c.transports.join(", ") : "",
    c.inbound_protocols?.length
      ? `入口协议 ${c.inbound_protocols.join(", ")}`
      : "",
    c.inbounds?.length ? `入口 ${c.inbounds.join(", ")}` : "",
    c.source_cidrs?.length ? `来源 ${c.source_cidrs.join(", ")}` : "",
    c.process
      ? `本机进程 ${c.process.command ?? c.process.pid ?? c.process.cgroup_id}`
      : "",
  ].filter(Boolean);
  return parts.join(" · ") || "空条件：匹配全部流量";
}
const blankRule: Rule = {
  id: "",
  enabled: false,
  conditions: {},
  action: { kind: "reject" },
};
type RoutingContext = {
  destination_host: string;
  destination_port: number;
  transport: "tcp" | "udp";
  source_ip: string | null;
  source_port?: number | null;
  inbound: string | null;
  inbound_protocol: string | null;
  observed_domains: string[];
  origin: "explicit" | "linux_local" | "gateway";
  process: { pid: number; command: string; cgroup_id: number } | null;
};
const defaultContext: RoutingContext = {
  destination_host: "example.com",
  destination_port: 443,
  transport: "tcp",
  source_ip: null,
  inbound: null,
  inbound_protocol: null,
  observed_domains: [],
  origin: "explicit",
  process: null,
};
const message = (cause: unknown) =>
  cause instanceof Error ? cause.message : "操作失败";

export default function RulesPage({
  refreshService,
}: {
  refreshService: () => Promise<void>;
}) {
  const [data, setData] = useState<Overview>();
  const [error, setError] = useState("");
  const [info, setInfo] = useState("");
  const [busy, setBusy] = useState(false);
  const [page, setPage] = useState(0);
  const [editor, setEditor] = useState<
    {
      revision: number;
      base: number;
    } & (
      | { mode: "rule"; original: string | null; value: Rule }
      | { mode: "policy"; original: null; value: Policy }
    )
  >();
  const [context, setContext] = useState(defaultContext);
  const [simulation, setSimulation] = useState<Simulation>();
  const mounted = useRef(true);
  const sequence = useRef(0);
  useEffect(() => {
    mounted.current = true;
    const controller = new AbortController();
    const request = ++sequence.current;
    const raw = new URLSearchParams(location.search).get("revision");
    const revision =
      raw &&
      /^\d+$/.test(raw) &&
      Number.isSafeInteger(Number(raw)) &&
      Number(raw) > 0
        ? Number(raw)
        : undefined;
    if (raw && !revision)
      setError("规则版本必须是正整数，已读取当前运行版本。");
    getJson<Overview>(
      `api/v1/rules${revision ? `?revision=${revision}` : ""}`,
      controller.signal,
    )
      .then((value) => {
        if (mounted.current && request === sequence.current) setData(value);
      })
      .catch((cause) => {
        if (
          !controller.signal.aborted &&
          mounted.current &&
          request === sequence.current
        )
          setError(message(cause));
      });
    return () => {
      mounted.current = false;
      controller.abort();
    };
  }, []);
  useEffect(() => {
    if (!editor) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [editor]);
  async function load(revision?: number) {
    const request = ++sequence.current;
    const value = await getJson<Overview>(
      `api/v1/rules${revision ? `?revision=${revision}` : ""}`,
    );
    if (mounted.current && request === sequence.current) {
      setData(value);
      setPage((current) =>
        Math.min(
          current,
          Math.max(0, Math.ceil(value.configuration.rules.length / 50) - 1),
        ),
      );
      setSimulation(undefined);
      const url = new URL(location.href);
      if (revision) url.searchParams.set("revision", String(revision));
      else url.searchParams.delete("revision");
      history.replaceState(null, "", url);
    }
    return value;
  }
  async function action(run: () => Promise<void>) {
    setBusy(true);
    setError("");
    setInfo("");
    try {
      await run();
    } catch (cause) {
      if (mounted.current) setError(message(cause));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  async function candidate(path: string, body: unknown) {
    const value = await postJson<Revision>(path, body);
    if (!mounted.current) return;
    setEditor(undefined);
    setInfo(`候选 r${value.id} 已保存，需要显式激活。`);
    await load(value.id);
  }
  async function edit(mode: "rule" | "policy", id?: string) {
    if (!data) return;
    const value = await load(data.viewed_revision);
    const selected = id
      ? value.configuration.rules.find((r) => r.id === id)
      : blankRule;
    if (!selected) throw new Error("规则已移除，请刷新后重试");
    if (mounted.current)
      setEditor(
        mode === "policy"
          ? {
              mode,
              original: null,
              revision: value.running_revision,
              base: value.viewed_revision,
              value: value.configuration,
            }
          : {
              mode,
              original: id ?? null,
              revision: value.running_revision,
              base: value.viewed_revision,
              value: selected,
            },
      );
  }
  async function save() {
    if (!editor) return;
    // Switching a tagged action can leave an earlier target in the draft.
    const cleanAction = (action: Action): Action =>
      action.kind === "node" || action.kind === "group"
        ? action
        : { kind: action.kind };
    const value =
      editor.mode === "policy"
        ? {
            ...editor.value,
            default_action: cleanAction(editor.value.default_action),
          }
        : { ...editor.value, action: cleanAction(editor.value.action) };
    await candidate(
      `api/v1/rules/${editor.mode === "policy" ? "save" : "upsert"}`,
      {
        [editor.mode === "policy" ? "configuration" : "rule"]: value,
        expected_revision: editor.revision,
        base_revision: editor.base,
      },
    );
  }
  async function move(index: number, delta: number) {
    if (!data) return;
    const ids = data.configuration.rules.map((r) => r.id);
    [ids[index], ids[index + delta]] = [ids[index + delta], ids[index]];
    await candidate("api/v1/rules/reorder", {
      ids,
      expected_revision: data.running_revision,
      base_revision: data.viewed_revision,
    });
  }
  async function apply() {
    if (!data) return;
    const value = await postJson<{ state: string; running_revision: number }>(
      "api/v1/rules/apply",
      {
        revision: data.viewed_revision,
        expected_revision: data.running_revision,
        defer_until_restart: false,
      },
    );
    if (!mounted.current) return;
    setInfo(`激活结果 ${value.state} · 实际运行 r${value.running_revision}`);
    await Promise.all([load(), refreshService()]);
  }
  async function simulate() {
    if (!data) return;
    const value = await postJson<Simulation>("api/v1/rules/simulate", {
      revision: data.viewed_revision,
      context,
    });
    if (mounted.current) setSimulation(value);
  }
  const needsRebase = Boolean(
    data &&
    data.viewed_revision !== data.running_revision &&
    data.viewed_parent_revision !== data.running_revision,
  );
  const canMutate = Boolean(
    data &&
    !busy &&
    !editor &&
    !needsRebase &&
    !data.other_configuration_changes &&
    data.desired_revision === data.running_revision,
  );
  const canApply =
    canMutate && data?.viewed_revision !== data?.running_revision;
  const knownViews = new Set(
    data ? [data.running_revision, ...data.candidates.map((c) => c.id)] : [],
  );
  return (
    <div className="space-y-3">
      {error && (
        <p
          role="alert"
          className="panel panel-body border-red-200 text-sm text-red-700 dark:text-red-300"
        >
          {error}
        </p>
      )}
      {info && (
        <p
          role="status"
          className="panel panel-body border-emerald-200 text-sm text-emerald-700 dark:text-emerald-300"
        >
          {info}
        </p>
      )}
      <section className="panel panel-body space-y-2.5">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h2 className="flex items-center gap-2 font-semibold">
              <Waypoints size={18} />
              规则与运行版本
            </h2>
            <p className="muted mt-2 text-xs leading-6">
              {data
                ? `运行 r${data.running_revision} · 期望 r${data.desired_revision} · 正在查看 r${data.viewed_revision}`
                : "正在读取规则…"}
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn"
              disabled={!canMutate}
              onClick={() => void action(() => edit("rule"))}
            >
              <Plus size={14} />
              新增规则
            </button>
            <a className="btn" href="#rule-simulation">
              规则模拟
            </a>
            <button
              className="btn"
              disabled={!canMutate}
              onClick={() => void action(() => edit("policy"))}
            >
              <Settings2 size={14} />
              编辑规则策略
            </button>
            <button
              className="btn"
              disabled={busy || Boolean(editor)}
              onClick={() =>
                void action(async () => {
                  await load(data?.viewed_revision);
                })
              }
            >
              <RefreshCw size={14} />
              刷新规则
            </button>
          </div>
        </div>
        <p className="muted text-xs leading-6">
          按顺序匹配首条规则。候选激活后对新连接生效。
        </p>
        {data && (
          <>
            <div className="flex flex-wrap items-end gap-3">
              <label className="form-field min-w-48 flex-1">
                查看版本
                <select
                  className="field"
                  value={String(data.viewed_revision)}
                  disabled={busy || Boolean(editor)}
                  onChange={(event) => {
                    const id = Number(event.target.value);
                    setPage(0);
                    void action(async () => {
                      await load(id === data.running_revision ? undefined : id);
                    });
                  }}
                >
                  <option value={data.running_revision}>
                    运行 r{data.running_revision}
                  </option>
                  {data.candidates.map((c) => (
                    <option key={c.id} value={c.id}>
                      候选 r{c.id} · 基于 r{c.parent_id}
                    </option>
                  ))}
                  {!knownViews.has(data.viewed_revision) && (
                    <option value={data.viewed_revision}>
                      历史 r{data.viewed_revision}
                    </option>
                  )}
                </select>
              </label>
              <button
                className="btn-primary"
                disabled={!canApply}
                onClick={() => void action(apply)}
              >
                激活查看的候选
              </button>
            </div>
            <p className="text-xs">
              运行默认动作：
              {actionName(data.active_configuration.default_action)} ·
              查看默认动作：{actionName(data.configuration.default_action)}
            </p>
            <p className="muted text-xs">
              {data.configuration.rules.length} 条规则 · DNS 观测推断
              {data.configuration.use_dns_observations
                ? "已显式开启，仅使用单个未过期候选"
                : "默认停用"}
            </p>
            {data.viewed_revision !== data.running_revision && (
              <div className="rounded-xl border border-amber-200 bg-amber-50 p-4 text-xs leading-6 text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
                <p>正在查看候选或历史版本，运行规则尚未被本次查看改变。</p>
                <p className="break-all">
                  变更路径：
                  {data.changes.map((c) => c.path).join("，") || "无内容变化"}
                </p>
                <details className="mt-2">
                  <summary className="cursor-pointer">查看变更内容</summary>
                  <pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap break-all text-[11px]">
                    {JSON.stringify(data.changes, null, 2)}
                  </pre>
                </details>
              </div>
            )}
            {(needsRebase || data.other_configuration_changes) && (
              <div className="space-y-3 text-xs leading-6">
                <p>
                  查看版本基于旧配置或含其他设置变更。先将这里的规则基于当前运行版本另存，其他设置保持当前值。
                </p>
                <button
                  className="btn"
                  disabled={
                    busy ||
                    Boolean(editor) ||
                    data.desired_revision !== data.running_revision
                  }
                  onClick={() =>
                    void action(() =>
                      candidate("api/v1/rules/save", {
                        configuration: data.configuration,
                        expected_revision: data.running_revision,
                        base_revision: null,
                      }),
                    )
                  }
                >
                  基于当前版本另存候选
                </button>
              </div>
            )}
            {data.desired_revision !== data.running_revision && (
              <p className="text-xs text-amber-700 dark:text-amber-300">
                有待重启配置，请先在配置工作台处理版本状态，再保存规则候选。
              </p>
            )}
            <div className="flex flex-wrap gap-2">
              <Badge active={data.capabilities.kernel_rules_ready}>
                {data.capabilities.kernel_rules_ready
                  ? `内核规则 r${data.capabilities.kernel_rule_revision}`
                  : "内核规则尚未接管"}
              </Badge>
              <Badge active={data.capabilities.local_process_available}>
                {data.capabilities.local_process_available
                  ? "本机进程来源可用"
                  : "本机进程来源不可用"}
              </Badge>
            </div>
            {!data.capabilities.local_process_available && (
              <p className="muted text-xs leading-6">
                当前不能启用本机进程条件，也不能识别远端代理客户端或网关后的进程。DNS
                捕获状态与完整规则数据面分别报告。
              </p>
            )}
          </>
        )}
      </section>
      {editor && (
        <section className="panel panel-body space-y-4" aria-label="规则编辑器">
          <h2 className="font-semibold">
            {editor.mode === "policy"
              ? "编辑规则策略"
              : editor.original
                ? `编辑规则 ${editor.original}`
                : "新增规则"}{" "}
            · 运行基准 r{editor.revision} / 内容 r{editor.base}
          </h2>
          <p className="muted text-xs leading-6">
            {editor.mode === "policy"
              ? "没有规则命中时使用默认动作。候选版本需显式激活后才接管流量。"
              : "同类条件满足任意一项即可，不同类别需要同时满足。空条件匹配全部流量。新规则默认停用。"}
          </p>
          <ConfigForm
            label="规则配置"
            onSave={() => {
              if (!busy) void action(save);
            }}
          >
            {editor.mode === "policy" ? (
              <ConfigFields
                fields={routingFields}
                value={editor.value}
                onChange={(value) => setEditor({ ...editor, value })}
              />
            ) : (
              <ConfigFields
                fields={ruleFields(Boolean(editor.original))}
                value={editor.value}
                onChange={(value) => setEditor({ ...editor, value })}
              />
            )}
            <div className="flex flex-wrap gap-2">
              <button className="btn-primary" type="submit" disabled={busy}>
                保存为候选
              </button>
              <button
                className="btn"
                type="button"
                disabled={busy}
                onClick={() => setEditor(undefined)}
              >
                取消编辑
              </button>
            </div>
          </ConfigForm>
        </section>
      )}
      <section className="space-y-2" aria-label="有序规则">
        {data?.configuration.rules
          .slice(page * 50, (page + 1) * 50)
          .map((rule, localIndex) => {
            const index = page * 50 + localIndex;
            const unsupported = Boolean(
              rule.conditions.process &&
              !data.capabilities.local_process_available,
            );
            return (
              <article
                key={rule.id}
                className="panel panel-body min-w-0"
                tabIndex={0}
                onKeyDown={(event) => {
                  if (
                    !canMutate ||
                    !event.altKey ||
                    !["ArrowUp", "ArrowDown"].includes(event.key)
                  )
                    return;
                  event.preventDefault();
                  const delta = event.key === "ArrowUp" ? -1 : 1;
                  if (
                    index + delta >= 0 &&
                    index + delta < data.configuration.rules.length
                  )
                    void action(() => move(index, delta));
                }}
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h3 className="break-all font-semibold">
                      {index + 1}. {rule.id}
                    </h3>
                    <p className="mt-2 text-sm">{actionName(rule.action)}</p>
                  </div>
                  <Badge active={rule.enabled}>
                    {rule.enabled ? "已启用" : "已停用"}
                  </Badge>
                </div>
                <p className="muted mt-3 break-all text-xs leading-6">
                  {description(rule.conditions)}
                </p>
                <div className="mt-4 flex flex-wrap gap-2">
                  <button
                    className="btn"
                    disabled={!canMutate || index === 0}
                    aria-label={`上移规则 ${rule.id}`}
                    onClick={() => void action(() => move(index, -1))}
                  >
                    <ArrowUp size={14} />
                  </button>
                  <button
                    className="btn"
                    disabled={
                      !canMutate ||
                      index === data.configuration.rules.length - 1
                    }
                    aria-label={`下移规则 ${rule.id}`}
                    onClick={() => void action(() => move(index, 1))}
                  >
                    <ArrowDown size={14} />
                  </button>
                  <button
                    className="btn"
                    disabled={!canMutate}
                    onClick={() => void action(() => edit("rule", rule.id))}
                  >
                    编辑 {rule.id}
                  </button>
                  <button
                    className="btn"
                    disabled={!canMutate || (!rule.enabled && unsupported)}
                    onClick={() =>
                      void action(() =>
                        candidate("api/v1/rules/toggle", {
                          id: rule.id,
                          enabled: !rule.enabled,
                          expected_revision: data.running_revision,
                          base_revision: data.viewed_revision,
                        }),
                      )
                    }
                  >
                    {rule.enabled ? "停用" : "启用"} {rule.id}
                  </button>
                  <button
                    className="btn"
                    disabled={!canMutate}
                    onClick={() =>
                      void action(() =>
                        candidate("api/v1/rules/delete", {
                          id: rule.id,
                          expected_revision: data.running_revision,
                          base_revision: data.viewed_revision,
                        }),
                      )
                    }
                  >
                    删除 {rule.id}
                  </button>
                </div>
                {unsupported && (
                  <p className="muted mt-3 text-xs">
                    进程来源不可用，可保存停用规则；启用时服务端会拒绝发布。
                  </p>
                )}
              </article>
            );
          })}
        {data && data.configuration.rules.length === 0 && (
          <p className="panel panel-body text-sm muted">
            当前查看版本没有规则，使用默认动作。
          </p>
        )}
        {data && data.configuration.rules.length > 50 && (
          <div className="flex flex-wrap items-center gap-3 text-xs">
            <button
              className="btn"
              disabled={page === 0}
              onClick={() => setPage(page - 1)}
            >
              上一页规则
            </button>
            <span>
              第 {page + 1} 页 /{" "}
              {Math.ceil(data.configuration.rules.length / 50)} 页
            </span>
            <button
              className="btn"
              disabled={(page + 1) * 50 >= data.configuration.rules.length}
              onClick={() => setPage(page + 1)}
            >
              下一页规则
            </button>
          </div>
        )}
        <p className="muted text-xs">
          聚焦规则卡片可使用 Alt + ↑ / ↓ 调整顺序；调整仅保存新候选。
        </p>
      </section>
      <section
        id="rule-simulation"
        className="panel panel-body space-y-3"
        aria-label="规则模拟"
      >
        <h2 className="flex items-center gap-2 font-semibold">
          <Search size={17} />
          模拟当前查看的规则
        </h2>
        <p className="muted text-xs leading-6">
          不连接目标、不解析新的域名、不购买节点，不推进轮询或创建会话。候选使用当前非规则设置与实时健康/租约。显式域名优先，CIDR
          需要字面目标 IP；未提供进程不能推断。
        </p>
        <ConfigForm
          label="规则模拟条件"
          onSave={() => {
            if (!busy && data && !editor) void action(simulate);
          }}
        >
          <ConfigFields
            fields={simulationFields}
            value={context}
            onChange={setContext}
          />
          <button
            className="btn-primary"
            type="submit"
            disabled={busy || !data || Boolean(editor)}
          >
            执行规则模拟
          </button>
        </ConfigForm>
        {simulation && (
          <div className="space-y-4" aria-label="规则模拟结果">
            <p className="break-all text-sm">
              规则版本 r{simulation.decision.configuration_revision} · 命中{" "}
              {simulation.decision.rule_id ?? "默认动作"} ·{" "}
              {actionName(simulation.decision.action)}
            </p>
            <p className="muted break-all text-xs">
              域名 {simulation.decision.domain ?? "未知"} · 来源{" "}
              {simulation.decision.domain_source} ·{" "}
              {simulation.health_evaluated_at_unix === null
                ? "未涉及节点准入"
                : `候选准入评估时刻 ${new Date(simulation.health_evaluated_at_unix * 1000).toLocaleString()}`}
            </p>
            {!simulation.action_supported && (
              <p className="text-xs text-amber-700 dark:text-amber-300">
                当前数据面不支持此传输动作，模拟匹配不代表可以转发。
              </p>
            )}
            {simulation.selection && (
              <div className="space-y-3">
                <p className="text-sm">
                  选择 {simulation.selection.selected ?? "无可用节点"} · 可用{" "}
                  {simulation.selection.eligible} / {simulation.selection.total}
                </p>
                <div className="max-h-72 overflow-auto">
                  <table className="w-full min-w-[480px] text-left text-xs">
                    <thead>
                      <tr>
                        <th className="p-2">节点</th>
                        <th className="p-2">协议</th>
                        <th className="p-2">候选解释</th>
                      </tr>
                    </thead>
                    <tbody>
                      {simulation.selection.candidates.map((node) => (
                        <tr
                          className="border-t border-console-border"
                          key={node.node_id}
                        >
                          <td className="p-2">{node.node_id}</td>
                          <td className="p-2">{node.protocol}</td>
                          <td className="p-2">
                            {node.eligible
                              ? "合格候选"
                              : node.reasons
                                  .map((r) => candidateReasons[r] ?? r)
                                  .join("；")}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                <p className="muted text-xs">
                  显示候选前 50 条，完整分页与固定范围见
                  <a
                    className="ml-1 text-console-accent-ink underline"
                    href="./groups.html"
                  >
                    节点分组
                  </a>
                  。
                </p>
              </div>
            )}
            <ol className="space-y-2">
              {simulation.decision.trace.map((item) => (
                <li
                  key={item.id}
                  className="rounded-lg bg-console-soft p-3 text-xs leading-6"
                >
                  <strong>{item.id}</strong> ·{" "}
                  {item.matched
                    ? "首条命中，后续不再计算"
                    : item.reasons.map((r) => ruleReasons[r] ?? r).join("；")}
                </li>
              ))}
            </ol>
            <details className="muted text-xs leading-6">
              <summary className="cursor-pointer">诊断说明</summary>
              {simulation.notes.map((note, index) => (
                <p key={index}>{note}</p>
              ))}
            </details>
          </div>
        )}
      </section>
      <ObservationPanel mode="rules" />
      {data && (
        <RuleImportPanel
          key={data.running_revision}
          revision={data.running_revision}
          locked={busy}
          viewed={data.viewed_revision}
          onImport={(document, checksum) =>
            candidate("api/v1/rules/import/apply", {
              document,
              checksum,
              expected_revision: data.running_revision,
            })
          }
        />
      )}
    </div>
  );
}
