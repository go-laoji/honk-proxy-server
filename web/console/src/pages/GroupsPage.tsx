import {
  type SelectionDecision as Decision,
  strategyLabels as strategies,
  candidateReasons as reasons,
} from "../selection";
import { useEffect, useRef, useState } from "react";
import { Layers3, Plus, RefreshCw, Search, Settings2 } from "lucide-react";
import { Badge, Stat } from "../components/ui";
import { getJson, postJson } from "../platform";

type Group = {
  id: string;
  strategy: string;
  include_all: boolean;
  members: string[];
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
    <div className="space-y-6">
      {error && (
        <p
          role="alert"
          className="panel border-red-200 p-4 text-sm text-red-700 dark:text-red-300"
        >
          {error}
        </p>
      )}
      {info && (
        <p
          role="status"
          className="panel border-emerald-200 p-4 text-sm text-emerald-700 dark:text-emerald-300"
        >
          {info}
        </p>
      )}
      <section className="panel p-5 sm:p-6">
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
        <p className="muted mt-4 text-xs leading-6">
          固定失败只允许显式回退范围；候选仍受健康、协议和剩余寿命限制。策略用于新连接，已有连接不迁移。未配置分组时沿用全池轮询。
        </p>
        {data && (
          <div className="mt-5 grid gap-3 sm:grid-cols-3">
            <Stat
              title="已配置分组"
              detail="成员、来源和标签统一过滤"
              icon={Layers3}
              value={String(data.configuration.groups.length)}
            />
            <Stat
              title="默认选择可用候选"
              detail="实际准入检查，尚未连接节点"
              icon={Search}
              value={String(data.default_selection.eligible)}
            />
            <Stat
              title="会话表上限 / 当前"
              detail="新代际重建，不延长租约"
              icon={Settings2}
              value={`${data.configuration.max_sessions} / ${data.default_selection.sessions}`}
            />
          </div>
        )}
      </section>
      {editor && (
        <section className="panel space-y-4 p-5" aria-label="分组编辑器">
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
              : "members 与 providers 取并集，再按 tags 和 protocols 过滤；include_all 明确选择全池。会话可配置 source_ip / source_destination 和 ttl_secs。"}
          </p>
          <label className="block text-xs font-medium">
            {editor.mode === "policy" ? "选择策略 JSON" : "分组 JSON"}
            <textarea
              className="input mt-2 min-h-80 w-full font-mono text-xs"
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
      <section className="grid gap-4 md:grid-cols-2" aria-label="分组列表">
        {data?.configuration.groups.map((item) => (
          <article key={item.id} className="panel min-w-0 p-5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h3 className="break-all font-semibold">{item.id}</h3>
              <Badge active>{strategies[item.strategy] ?? item.strategy}</Badge>
            </div>
            <dl className="mt-4 space-y-2 text-xs leading-6">
              <div>
                <dt className="muted inline">成员：</dt>
                <dd className="inline break-all">
                  {item.include_all
                    ? "全部节点"
                    : `${item.members.length} 个节点 / ${item.providers.length} 个来源`}
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
            <div className="mt-4 flex flex-wrap gap-2">
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
          <p className="panel p-6 text-sm text-slate-500">
            尚未配置分组，当前使用全池轮询。
          </p>
        )}
      </section>
      <section className="panel space-y-4 p-5 sm:p-6" aria-label="选点模拟">
        <h2 className="flex items-center gap-2 font-semibold">
          <Search size={17} />
          候选与只读模拟
        </h2>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <label className="text-xs">
            分组
            <select
              className="input mt-2 w-full"
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
          <label className="text-xs">
            入口 ID
            <input
              className="input mt-2 w-full"
              value={inbound}
              onChange={(event) => setInbound(event.target.value)}
              placeholder="可选"
            />
          </label>
          <label className="text-xs">
            源 IP
            <input
              className="input mt-2 w-full"
              value={source}
              onChange={(event) => setSource(event.target.value)}
              placeholder="可选；只查询已有会话"
            />
          </label>
          <label className="text-xs">
            目的主机
            <input
              className="input mt-2 w-full"
              value={host}
              onChange={(event) => setHost(event.target.value)}
              placeholder="example.com / IPv4 / IPv6"
            />
          </label>
          <label className="text-xs">
            目的端口
            <input
              className="input mt-2 w-full"
              type="number"
              min={1}
              max={65535}
              value={port}
              onChange={(event) => setPort(event.target.value)}
            />
          </label>
          <label className="text-xs">
            传输
            <select
              className="input mt-2 w-full"
              value={transport}
              onChange={(event) => setTransport(event.target.value)}
            >
              <option value="tcp">TCP</option>
              <option value="udp">UDP（当前节点均为 TCP-only）</option>
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
                  <tr className="border-b border-slate-200 text-slate-500 dark:border-slate-700">
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
                      className="border-b border-slate-100 dark:border-slate-800"
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
                          ? "无租约期限"
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
        <section className="panel space-y-4 p-5" aria-label="手动固定">
          <h2 className="font-semibold">手动固定 · {selectedGroup.id}</h2>
          <div className="grid gap-4 sm:grid-cols-2">
            <label className="text-xs">
              固定节点 ID
              <input
                className="input mt-2 w-full"
                value={pin}
                onChange={(event) => setPin(event.target.value)}
              />
            </label>
            <label className="text-xs">
              允许回退节点（逗号分隔）
              <input
                className="input mt-2 w-full"
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
