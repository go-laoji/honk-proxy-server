import {
  type SelectionDecision as Decision,
  strategyLabels as strategies,
  candidateReasons as reasons,
} from "../selection";
import { useEffect, useRef, useState } from "react";
import { Layers3, Plus, RefreshCw, Search, Settings2 } from "lucide-react";
import { Badge } from "../components/ui";
import { getJson, postJson } from "../platform";

type Group = {
  id: string;
  strategy: string;
  include_all: boolean;
  members: string[];
  groups: string[];
  score: {
    latency_weight: number;
    lifetime_weight: number;
    priority_weight: number;
    latency_ceiling_ms: number;
    lifetime_target_secs: number;
    missing_evidence_penalty: number;
  };
  providers: string[];
  tags: string[];
  protocols: string[];
  pinned_node: string | null;
  fallback_nodes: string[];
  weights: Record<string, number>;
  session: { key: string; ttl_secs: number } | null;
};
type Policy = {
  groups: Group[];
  default_group: string | null;
  inbound_groups: Record<string, string>;
  node_tags: Record<string, string[]>;
  provider_tags: Record<string, string[]>;
  max_sessions: number;
};
type Overview = {
  running_revision: number;
  configuration: Policy;
  recent_choices: Record<string, string>;
  default_selection: Decision;
};
const blankGroup: Group = {
  id: "new-group",
  strategy: "round_robin",
  include_all: false,
  members: [],
  groups: [],
  score: {
    latency_weight: 60,
    lifetime_weight: 30,
    priority_weight: 10,
    latency_ceiling_ms: 2000,
    lifetime_target_secs: 300,
    missing_evidence_penalty: 250,
  },
  providers: [],
  tags: [],
  protocols: [],
  pinned_node: null,
  fallback_nodes: [],
  weights: {},
  session: null,
};
const message = (cause: unknown) =>
  cause instanceof Error ? cause.message : "操作失败";

export default function GroupsPage({
  refreshService,
}: {
  refreshService: () => Promise<void>;
}) {
  const [data, setData] = useState<Overview>();
  const [error, setError] = useState("");
  const [info, setInfo] = useState("");
  const [busy, setBusy] = useState(false);
  const [editor, setEditor] = useState<{
    mode: "group" | "policy";
    revision: number;
    original: string | null;
    body: string;
  }>();
  const [group, setGroup] = useState(
    new URLSearchParams(window.location.search).get("group") ?? "",
  );
  const [inbound, setInbound] = useState("");
  const [source, setSource] = useState("");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("");
  const [transport, setTransport] = useState("tcp");
  const [pin, setPin] = useState("");
  const [fallback, setFallback] = useState("");
  const [decision, setDecision] = useState<Decision>();
  const [lastQuery, setLastQuery] = useState<Record<string, unknown>>();
  const mounted = useRef(true);
  const sequence = useRef(0);
  useEffect(() => {
    mounted.current = true;
    const controller = new AbortController();
    const request = ++sequence.current;
    getJson<Overview>("api/v1/groups", controller.signal)
      .then((value) => {
        if (mounted.current && request === sequence.current) {
          setData(value);
          const linked = value.configuration.groups.find((g) => g.id === group);
          setPin(linked?.pinned_node ?? "");
          setFallback(linked?.fallback_nodes.join(", ") ?? "");
        }
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
  async function refresh() {
    const request = ++sequence.current;
    const value = await getJson<Overview>("api/v1/groups");
    if (mounted.current && request === sequence.current) setData(value);
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
  async function edit(mode: "group" | "policy", id?: string) {
    const value = await refresh();
    const selected = id
      ? value.configuration.groups.find((g) => g.id === id)
      : blankGroup;
    if (!selected) throw new Error("分组已被其他配置移除，请刷新");
    if (mounted.current)
      setEditor({
        mode,
        revision: value.running_revision,
        original: id ?? null,
        body: JSON.stringify(
          mode === "policy" ? value.configuration : selected,
          null,
          2,
        ),
      });
  }
  async function changed(path: string, body: unknown, label: string) {
    const result = await postJson<{ running_revision: number; state: string }>(
      path,
      body,
    );
    if (!mounted.current) return;
    setEditor(undefined);
    setDecision(undefined);
    setInfo(
      `${label} · ${result.state} · 运行版本 r${result.running_revision}`,
    );
    const [value] = await Promise.all([refresh(), refreshService()]);
    if (
      mounted.current &&
      group &&
      !value.configuration.groups.some((g) => g.id === group)
    )
      chooseGroup("");
  }
  async function save() {
    if (!editor) return;
    const value: unknown = JSON.parse(editor.body);
    if (
      editor.mode === "group" &&
      editor.original &&
      (!value ||
        typeof value !== "object" ||
        !("id" in value) ||
        value.id !== editor.original)
    ) {
      throw new Error("编辑时请保留分组 ID；创建新分组请使用新增按钮");
    }
    await changed(
      `api/v1/groups/${editor.mode === "policy" ? "save" : "upsert"}`,
      {
        [editor.mode === "policy" ? "configuration" : "group"]: value,
        expected_revision: editor.revision,
      },
      "分组配置已保存",
    );
  }
  async function simulate(after?: string) {
    const query =
      after && lastQuery
        ? { ...lastQuery, after }
        : {
            group: group || null,
            inbound: inbound || null,
            source_ip: source || null,
            destination_host: host || null,
            destination_port: port ? Number(port) : null,
            transport,
            limit: 50,
          };
    const result = await postJson<Decision>("api/v1/groups/simulate", query);
    if (mounted.current) {
      setDecision(result);
      setLastQuery(query);
    }
  }
  const selectedGroup = data?.configuration.groups.find((g) => g.id === group);
  function chooseGroup(id: string) {
    const selected = data?.configuration.groups.find((g) => g.id === id);
    setGroup(id);
    setPin(selected?.pinned_node ?? "");
    setFallback(selected?.fallback_nodes.join(", ") ?? "");
    setDecision(undefined);
  }
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
      <section className="panel panel-body space-y-2">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h2 className="flex items-center gap-2 font-semibold">
              <Layers3 size={18} />
              节点分组与选点
            </h2>
            <p className="muted mt-2 text-xs leading-6">
              {data
                ? `运行版本 r${data.running_revision} · 默认组 ${data.configuration.default_group ?? "全池轮询"}`
                : "正在读取分组…"}
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn"
              disabled={busy || !data}
              onClick={() => void action(() => edit("group"))}
            >
              <Plus size={14} />
              新增分组
            </button>
            <a className="btn" href="#selection-simulation">
              选点模拟
            </a>
            <button
              className="btn"
              disabled={busy || !data}
              onClick={() => void action(() => edit("policy"))}
            >
              <Settings2 size={14} />
              编辑选择策略
            </button>
            <button
              className="btn"
              disabled={busy}
              onClick={() =>
                void action(async () => {
                  await refresh();
                })
              }
            >
              <RefreshCw size={14} />
              刷新分组
            </button>
          </div>
        </div>
        {data && (
          <dl className="muted flex flex-wrap gap-x-4 gap-y-1 text-xs tabular-nums">
            <div className="flex gap-1">
              <dt>分组</dt>
              <dd className="font-semibold">
                {data.configuration.groups.length}
              </dd>
            </div>
            <div className="flex gap-1">
              <dt>默认可用候选</dt>
              <dd className="font-semibold">
                {data.default_selection.eligible}
              </dd>
            </div>
            <div className="flex gap-1">
              <dt>会话</dt>
              <dd className="font-semibold">
                {data.default_selection.sessions} /{" "}
                {data.configuration.max_sessions}
              </dd>
            </div>
          </dl>
        )}
      </section>
      {editor && (
        <section className="panel panel-body space-y-4" aria-label="分组编辑器">
          <h2 className="font-semibold">
            {editor.mode === "policy"
              ? "选择策略与引用"
              : editor.original
                ? `编辑分组 ${editor.original}`
                : "新增分组"}{" "}
            · 基于 r{editor.revision}
          </h2>
          <p className="muted text-xs leading-6">
            {editor.mode === "policy"
              ? "配置默认组、入口绑定、节点和来源标签，以及会话表上限。删除被引用的分组前先移除引用。"
              : "members、providers 与 groups 子组取并集，再按 tags/protocols 过滤。订阅来源写 subscription:来源ID；子组保留过滤与固定范围，最多 8 层且拒绝循环。strategy=score 使用 score 权重与显式 weights 运营优先级；缺失证据逐项扣分，不推断成本或地域。"}
          </p>
          <label className="form-field">
            {editor.mode === "policy" ? "选择策略 JSON" : "分组 JSON"}
            <textarea
              className="field editor-field min-h-80"
              spellCheck={false}
              value={editor.body}
              onChange={(event) =>
                setEditor({ ...editor, body: event.target.value })
              }
            />
          </label>
          <div className="flex gap-2">
            <button
              className="btn-primary"
              disabled={busy}
              onClick={() => void action(save)}
            >
              保存并应用分组
            </button>
            <button
              className="btn"
              disabled={busy}
              onClick={() => setEditor(undefined)}
            >
              取消编辑
            </button>
          </div>
        </section>
      )}
      <section className="grid gap-2 md:grid-cols-2" aria-label="分组列表">
        {data?.configuration.groups.map((item) => (
          <article key={item.id} className="panel panel-body min-w-0">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h3 className="break-all font-semibold">{item.id}</h3>
              <Badge active>{strategies[item.strategy] ?? item.strategy}</Badge>
            </div>
            <dl className="mt-2 space-y-1 text-xs leading-5">
              <div>
                <dt className="muted inline">成员：</dt>
                <dd className="inline break-all">
                  {item.include_all
                    ? "全部节点"
                    : `${item.members.length} 个节点 / ${item.providers.length} 个来源 / ${item.groups.length} 个子组`}
                </dd>
              </div>
              <div>
                <dt className="muted inline">过滤：</dt>
                <dd className="inline break-all">
                  标签 {item.tags.join(", ") || "不限"} · 协议{" "}
                  {item.protocols.join(", ") || "不限"}
                </dd>
              </div>
              <div>
                <dt className="muted inline">固定：</dt>
                <dd className="inline break-all">
                  {item.pinned_node ?? "无"} · 回退{" "}
                  {item.fallback_nodes.join(", ") || "无"}
                </dd>
              </div>
              <div>
                <dt className="muted inline">最近实际选点：</dt>
                <dd className="inline break-all">
                  {data.recent_choices[item.id] ?? "本代际尚未选点"}
                </dd>
              </div>
              <div>
                <dt className="muted inline">会话：</dt>
                <dd className="inline">
                  {item.session
                    ? `${item.session.key} / 最长 ${item.session.ttl_secs}s，受租约准入限制`
                    : "停用"}
                </dd>
              </div>
            </dl>
            <div className="mt-2 flex flex-wrap gap-2">
              <button
                className="btn"
                disabled={busy}
                onClick={() => {
                  chooseGroup(item.id);
                }}
              >
                选择此组
              </button>
              <button
                className="btn"
                disabled={busy}
                onClick={() => void action(() => edit("group", item.id))}
              >
                编辑
              </button>
              <button
                className="btn"
                disabled={busy}
                onClick={() =>
                  void action(() =>
                    changed(
                      "api/v1/groups/delete",
                      {
                        group_id: item.id,
                        expected_revision: data.running_revision,
                      },
                      "分组已删除",
                    ),
                  )
                }
              >
                删除
              </button>
            </div>
          </article>
        ))}
        {data && data.configuration.groups.length === 0 && (
          <p className="panel panel-body text-sm muted">
            尚未配置分组，当前使用全池轮询。
          </p>
        )}
      </section>
      <section
        id="selection-simulation"
        className="panel panel-body space-y-3"
        aria-label="选点模拟"
      >
        <h2 className="flex items-center gap-2 font-semibold">
          <Search size={17} />
          候选与只读模拟
        </h2>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <label className="form-field">
            分组
            <select
              className="field"
              value={group}
              onChange={(event) => {
                chooseGroup(event.target.value);
              }}
            >
              <option value="">默认 / 按入口绑定</option>
              {data?.configuration.groups.map((g) => (
                <option key={g.id}>{g.id}</option>
              ))}
            </select>
          </label>
          <label className="form-field">
            入口 ID
            <input
              className="field"
              value={inbound}
              onChange={(event) => setInbound(event.target.value)}
              placeholder="可选"
            />
          </label>
          <label className="form-field">
            源 IP
            <input
              className="field"
              value={source}
              onChange={(event) => setSource(event.target.value)}
              placeholder="可选；只查询已有会话"
            />
          </label>
          <label className="form-field">
            目的主机
            <input
              className="field"
              value={host}
              onChange={(event) => setHost(event.target.value)}
              placeholder="example.com / IPv4 / IPv6"
            />
          </label>
          <label className="form-field">
            目的端口
            <input
              className="field"
              type="number"
              min={1}
              max={65535}
              value={port}
              onChange={(event) => setPort(event.target.value)}
            />
          </label>
          <label className="form-field">
            传输
            <select
              className="field"
              value={transport}
              onChange={(event) => setTransport(event.target.value)}
            >
              <option value="tcp">TCP</option>
              <option value="udp">UDP（SOCKS5 出口或显式直连）</option>
            </select>
          </label>
        </div>
        <button
          className="btn-primary"
          disabled={busy || !data}
          onClick={() => void action(() => simulate())}
        >
          模拟选点
        </button>
        <p className="muted text-xs leading-6">
          模拟不推进轮询、不建立会话、不请求供应商或连接节点。延迟优选只使用当前策略的新鲜主动探测证据；会话记录在配置代际切换或重启后重建。
        </p>
        {decision && (
          <div className="space-y-4" aria-label="选点结果">
            <p className="break-all text-sm">
              {decision.group ?? "全池"} ·{" "}
              {strategies[decision.strategy] ?? decision.strategy} · 选择{" "}
              {decision.selected ?? "无可用节点"} ·{" "}
              {reasons[decision.reason] ?? decision.reason}
            </p>
            <p className="muted text-xs">
              可用 {decision.eligible} / {decision.total} · 会话
              {decision.session_reused ? "已复用" : "未复用"}
              {decision.session_remaining_secs !== null
                ? ` / 剩余 ${decision.session_remaining_secs}s`
                : ""}
            </p>
            <div className="overflow-x-auto">
              <table className="w-full min-w-[640px] text-left text-xs">
                <thead>
                  <tr className="border-b border-console-border muted">
                    <th className="p-3">节点 / 来源</th>
                    <th className="p-3">协议 / 标签</th>
                    <th className="p-3">剩余寿命 / 延迟 / 权重</th>
                    <th className="p-3">候选解释</th>
                  </tr>
                </thead>
                <tbody>
                  {decision.candidates.map((node) => (
                    <tr
                      key={node.node_id}
                      className="border-b border-console-border"
                    >
                      <td className="p-3">
                        <p>{node.node_id}</p>
                        <p className="muted mt-1">
                          {node.provider_id ?? "静态节点"}
                        </p>
                      </td>
                      <td className="p-3">
                        {node.protocol}
                        <p className="muted mt-1">
                          {node.tags.join(", ") || "无标签"}
                        </p>
                      </td>
                      <td className="p-3">
                        {node.remaining_secs === null
                          ? "有效期未知 / 无租约"
                          : `${node.remaining_secs}s`}{" "}
                        /{" "}
                        {node.latency_ms === null
                          ? "未知"
                          : `${node.latency_ms}ms`}{" "}
                        / {node.weight}
                      </td>
                      <td className="p-3">
                        {node.eligible
                          ? "合格候选"
                          : node.reasons.map((r) => reasons[r] ?? r).join("；")}
                        {node.member_groups.length > 0 && (
                          <p className="muted mt-2">
                            子组：{node.member_groups.join(", ")}
                          </p>
                        )}
                        {node.score && (
                          <details className="mt-2">
                            <summary>
                              Score {node.score.total} · 分量与证据
                            </summary>
                            <ul className="mt-2 space-y-2">
                              {node.score.components.map((part) => (
                                <li key={part.source}>
                                  {(
                                    {
                                      fresh_health_probe_latency_ms:
                                        "新鲜健康延迟 ms",
                                      admission_remaining_lifetime_secs:
                                        "准入剩余寿命 s",
                                      configured_operator_priority:
                                        "显式运营优先级",
                                    } as Record<string, string>
                                  )[part.source] ?? part.source}
                                  ：{part.observed ?? "缺少证据"}；归一{" "}
                                  {part.normalized ?? "未知"} /1000；权重{" "}
                                  {part.weight}；贡献 {part.contribution}
                                  ；缺证据扣分 {part.missing_penalty}
                                </li>
                              ))}
                            </ul>
                          </details>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            {decision.candidates.length === 0 && (
              <p className="muted text-sm">当前页没有节点。</p>
            )}
            {decision.next_after && (
              <button
                className="btn"
                disabled={busy}
                onClick={() =>
                  void action(() => simulate(decision.next_after ?? undefined))
                }
              >
                下一页候选
              </button>
            )}
          </div>
        )}
      </section>
      {selectedGroup && data && (
        <section className="panel panel-body space-y-4" aria-label="手动固定">
          <h2 className="font-semibold">手动固定 · {selectedGroup.id}</h2>
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="form-field">
              固定节点 ID
              <input
                className="field"
                value={pin}
                onChange={(event) => setPin(event.target.value)}
              />
            </label>
            <label className="form-field">
              允许回退节点（逗号分隔）
              <input
                className="field"
                value={fallback}
                onChange={(event) => setFallback(event.target.value)}
              />
            </label>
          </div>
          <p className="muted text-xs leading-6">
            回退节点必须仍属于本组且满足准入。取消固定会清除回退列表；fixed
            策略会切换为轮询。不会迁移已建立连接。
          </p>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn-primary"
              disabled={busy || !pin.trim()}
              onClick={() =>
                void action(() =>
                  changed(
                    "api/v1/groups/pin",
                    {
                      group_id: selectedGroup.id,
                      node_id: pin.trim(),
                      fallback_nodes: fallback
                        .split(",")
                        .map((v) => v.trim())
                        .filter(Boolean),
                      expected_revision: data.running_revision,
                    },
                    "固定已更新",
                  ),
                )
              }
            >
              应用固定
            </button>
            <button
              className="btn"
              disabled={busy || !selectedGroup.pinned_node}
              onClick={() =>
                void action(() =>
                  changed(
                    "api/v1/groups/pin",
                    {
                      group_id: selectedGroup.id,
                      node_id: null,
                      fallback_nodes: [],
                      expected_revision: data.running_revision,
                    },
                    "固定已取消",
                  ),
                )
              }
            >
              取消固定
            </button>
          </div>
        </section>
      )}
    </div>
  );
}
