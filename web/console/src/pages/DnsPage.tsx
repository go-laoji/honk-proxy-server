import { useEffect, useRef, useState } from "react";
import {
  Database,
  Network,
  RefreshCw,
  Search,
  Settings2,
  Trash2,
} from "lucide-react";
import { Badge, Stat } from "../components/ui";
import { getJson, postJson } from "../platform";

type Purpose = "bootstrap" | "provider" | "health" | "business";
type Upstream = {
  id: string;
  protocol: string;
  addresses: string[];
  server_name: string | null;
  https_path: string | null;
  ca_certificate: string | null;
  proxy_id: string | null;
  proxy_bootstrap_ips: string[];
};
type Policy = {
  enabled: boolean;
  upstreams: Upstream[];
  routes: {
    id: string;
    upstream: string;
    domain: string | null;
    suffix: boolean;
    query_types: string[];
    purposes: Purpose[];
  }[];
  default_upstream: string;
  listen: string | null;
  capture: { object: string; cgroup: string; port: number } | null;
  timeout_ms: number;
  max_parallel: number;
  cache_entries: number;
  cache_bytes: number;
};
type Entry = {
  id: string;
  name: string;
  query_type: string;
  purpose: Purpose;
  upstream: string;
  egress: string | null;
  remaining_ttl: number;
  negative: boolean;
  bytes: number;
};
type Overview = {
  running_revision: number;
  configuration: Policy;
  listener: string | null;
  linux_capture_active: boolean;
  linux_capture: {
    cgroup: string;
    port: number;
    peers: number;
    program_ids: number[];
    object_sha256: string;
    stats: {
      family: string;
      redirected: number;
      restored: number;
      errors: number;
    }[];
  } | null;
  cache: {
    entries: Entry[];
    next_after: string | null;
    total: number;
    bytes: number;
    epoch: number;
    in_flight: number;
    system_workers: number;
    system_queued: number;
    system_worker_limit: number;
    hits: number;
    misses: number;
    merged: number;
    failures: number;
  };
};
type RecordView = {
  name: string;
  query_type: string;
  ttl: number;
  value: string;
};
type Lookup = {
  name: string;
  query_type: string;
  purpose: Purpose;
  upstream: string;
  route: string | null;
  protocol: string;
  egress: string | null;
  source: string;
  response_code: string;
  elapsed_ms: number;
  remaining_ttl: number | null;
  answers: RecordView[];
  authorities: RecordView[];
  dnssec_validated_locally: boolean;
};
type Context = {
  ip: string;
  domains: string[];
  source: string;
  ambiguous: boolean;
  mappings?: {
    domain: string;
    upstream: string;
    remaining_ttl: number;
    observed_age_ms: number;
    source: string;
  }[];
  mappings_truncated?: boolean;
};
const purposes: Record<Purpose, string> = {
  bootstrap: "节点引导",
  provider: "供应商接口",
  health: "健康检测",
  business: "业务查询",
};
const queryTypes = [
  "A",
  "AAAA",
  "CNAME",
  "MX",
  "NS",
  "SOA",
  "TXT",
  "SRV",
  "PTR",
  "CAA",
  "HTTPS",
  "SVCB",
  "TLSA",
  "DS",
  "DNSKEY",
];
const message = (cause: unknown) =>
  cause instanceof Error ? cause.message : "操作失败";

export default function DnsPage({
  refreshService,
}: {
  refreshService: () => Promise<void>;
}) {
  const [data, setData] = useState<Overview>();
  const [after, setAfter] = useState("");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [info, setInfo] = useState("");
  const [name, setName] = useState("example.com");
  const [kind, setKind] = useState("A");
  const [purpose, setPurpose] = useState<Purpose>("business");
  const [lookup, setLookup] = useState<Lookup>();
  const [domain, setDomain] = useState("");
  const [ip, setIp] = useState("");
  const [context, setContext] = useState<Context>();
  const [editor, setEditor] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const mounted = useRef(true);
  const sequence = useRef(0);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    const request = ++sequence.current;
    getJson<Overview>(
      `api/v1/dns?limit=50${after ? `&after=${encodeURIComponent(after)}` : ""}`,
      controller.signal,
    )
      .then((value) => {
        if (!controller.signal.aborted && request === sequence.current) {
          setData(value);
          setLoadError("");
        }
      })
      .catch((cause) => {
        if (!controller.signal.aborted && request === sequence.current)
          setLoadError(message(cause));
      });
    return () => controller.abort();
  }, [after]);
  async function refresh() {
    const request = ++sequence.current;
    const value = await getJson<Overview>(
      `api/v1/dns?limit=50${after ? `&after=${encodeURIComponent(after)}` : ""}`,
    );
    if (mounted.current && request === sequence.current) {
      setData(value);
      setLoadError("");
    }
  }
  async function action(label: string, run: () => Promise<void>) {
    setBusy(label);
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
  async function edit() {
    const value = await getJson<Overview>("api/v1/dns?limit=1");
    if (mounted.current) {
      setRevision(value.running_revision);
      setEditor(JSON.stringify(value.configuration, null, 2));
    }
  }
  async function save() {
    const result = await postJson<{ state: string; running_revision: number }>(
      "api/v1/dns/save",
      {
        configuration: JSON.parse(editor ?? ""),
        expected_revision: revision,
      },
    );
    if (!mounted.current) return;
    setEditor(null);
    setInfo(
      `DNS 策略已保存 · ${result.state} · 运行版本 r${result.running_revision}`,
    );
    await Promise.all([refresh(), refreshService()]);
  }
  return (
    <div className="space-y-3">
      {(error || loadError) && (
        <p
          role="alert"
          className="rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-200"
        >
          {error || loadError}
        </p>
      )}
      {info && (
        <p
          role="status"
          className="rounded-xl bg-console-accent-soft p-4 text-sm text-console-accent-ink"
        >
          {info}
        </p>
      )}
      {!data ? (
        <p className="panel p-10 text-center text-sm">正在读取 DNS…</p>
      ) : (
        <>
          <nav aria-label="DNS 页内导航" className="flex flex-wrap gap-2">
            <a className="btn" href="#dns-routing">
              上游与路由
            </a>
            <a className="btn" href="#dns-query">
              查询诊断
            </a>
            <a className="btn" href="#dns-cache">
              缓存
            </a>
            <a className="btn" href="#dns-context">
              域名上下文
            </a>
          </nav>
          <div className="metric-grid">
            <Stat
              title="DNS 服务"
              value={data.configuration.enabled ? "已启用" : "停用"}
              detail={
                data.listener
                  ? `监听 ${data.listener}`
                  : data.linux_capture
                    ? `Linux 双栈捕获端口 ${data.linux_capture.port}`
                    : "未启动 DNS 监听"
              }
              icon={Network}
            />
            <Stat
              title="缓存条目"
              value={`${data.cache.total} / ${data.configuration.cache_entries}`}
              detail={`${Math.ceil(data.cache.bytes / 1024)} KiB · 代际 ${data.cache.epoch}`}
              icon={Database}
            />
            <Stat
              title="命中 / 在途合并"
              value={`${data.cache.hits} / ${data.cache.merged}`}
              detail={`上游查询 ${data.cache.misses} · 失败 ${data.cache.failures}`}
              icon={Search}
            />
            <Stat
              title="解析并发"
              value={`${data.cache.in_flight} / ${data.configuration.max_parallel}`}
              detail={`请求期限 ${data.configuration.timeout_ms} ms · r${data.running_revision}${data.configuration.upstreams.some((up) => up.protocol === "system") ? ` · 系统解析 ${data.cache.system_workers}/${data.cache.system_worker_limit} · 排队 ${data.cache.system_queued}` : ""}`}
              icon={RefreshCw}
            />
          </div>
          <section
            id="dns-routing"
            className="panel panel-body"
            aria-label="DNS 上游与路由"
          >
            <div className="flex flex-wrap items-center justify-between gap-3">
              <h2 className="font-semibold">DNS 上游与路由</h2>
              <div className="flex gap-2">
                <button
                  className="btn"
                  disabled={!!busy}
                  onClick={() => void action("refresh", refresh)}
                >
                  <RefreshCw size={15} />
                  刷新
                </button>
                <button
                  className="btn"
                  disabled={!!busy}
                  onClick={() => void action("edit", edit)}
                >
                  <Settings2 size={15} />
                  编辑 DNS 策略
                </button>
              </div>
            </div>
            <div className="mt-4 grid gap-3 lg:grid-cols-2">
              {data.configuration.upstreams.map((up) => (
                <div
                  key={up.id}
                  className="rounded-xl bg-console-soft p-4 text-sm"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <strong>{up.id}</strong>
                    <Badge active>{up.protocol.toUpperCase()}</Badge>
                    {up.id === data.configuration.default_upstream && (
                      <span className="muted text-xs">默认上游</span>
                    )}
                  </div>
                  <p className="mt-2 break-all">
                    {up.protocol === "system"
                      ? "系统解析 · 无 wire TTL，结果不缓存"
                      : up.addresses.join("、")}
                  </p>
                  {up.server_name && (
                    <p className="muted mt-1 break-all text-xs">
                      证书名称 {up.server_name}
                      {up.https_path ?? ""}
                    </p>
                  )}
                  <p className="muted mt-2 text-xs">
                    {up.proxy_id
                      ? `经静态代理 ${up.proxy_id} · 引导 IP ${up.proxy_bootstrap_ips.join("、") || "节点为 IP"}`
                      : "直接访问 DNS 上游"}
                  </p>
                </div>
              ))}
            </div>
            <div className="mt-4 space-y-2 text-xs">
              {data.configuration.routes.length ? (
                data.configuration.routes.map((route) => (
                  <p
                    key={route.id}
                    className="break-all rounded-lg border border-console-border p-3"
                  >
                    <strong>{route.id}</strong> ·{" "}
                    {route.domain
                      ? `${route.suffix ? "域名及子域" : "精确域名"} ${route.domain}`
                      : "全部域名"}{" "}
                    · {route.query_types.join("/") || "全部类型"} ·{" "}
                    {route.purposes.map((p) => purposes[p]).join("/") ||
                      "全部用途"}{" "}
                    → {route.upstream}
                  </p>
                ))
              ) : (
                <p className="muted">
                  无特定路由，使用默认上游。设置路由后按排列顺序首次匹配。
                </p>
              )}
            </div>
            {editor !== null && (
              <div className="mt-5 space-y-3">
                <label className="form-field">
                  DNS 策略 JSON · 基于配置 r{revision}
                  <textarea
                    id="dns-policy"
                    className="field editor-field min-h-96"
                    value={editor}
                    onChange={(e) => setEditor(e.target.value)}
                    spellCheck={false}
                  />
                </label>
                <p className="muted text-xs leading-6">
                  监听模式需使用显式 IP 地址的 UDP、TCP、TLS 或 HTTPS
                  上游。TLS/HTTPS 验证
                  server_name；代理出口只可引用已启用的静态节点，域名节点需要
                  proxy_bootstrap_ips。保存前检查配置、信任证书和监听端口，其他用户已改配置时会提示冲突。
                </p>
                <p className="muted text-xs leading-6">
                  Linux DNS 捕获配置在 capture 字段填写 object（自有 eBPF
                  对象绝对路径）、cgroup（已有 cgroup v2
                  路径）、port（专用端口，默认
                  1053）。启用前服务会验证内核并准备监听；运行中修改或关闭捕获需要重启
                  Daemon，上游、路由和缓存策略可热更新。
                </p>
                <div className="flex gap-2">
                  <button
                    className="btn-primary"
                    disabled={!!busy}
                    onClick={() => void action("save", save)}
                  >
                    保存并应用 DNS
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
          <section
            id="dns-query"
            className="panel panel-body"
            aria-label="DNS 查询诊断"
          >
            <h2 className="font-semibold">查询诊断</h2>
            <form
              className="mt-4 flex flex-wrap items-end gap-3"
              onSubmit={(e) => {
                e.preventDefault();
                void action("resolve", async () => {
                  const result = await postJson<Lookup>("api/v1/dns/resolve", {
                    name,
                    query_type: kind,
                    purpose,
                  });
                  if (mounted.current) setLookup(result);
                  await refresh();
                });
              }}
            >
              <label className="form-field flex-1 basis-52">
                查询域名
                <input
                  className="field"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  maxLength={253}
                  required
                />
              </label>
              <label className="form-field">
                记录类型
                <select
                  className="field"
                  value={kind}
                  onChange={(e) => setKind(e.target.value)}
                >
                  {queryTypes.map((type) => (
                    <option key={type}>{type}</option>
                  ))}
                </select>
              </label>
              <label className="form-field">
                解析用途
                <select
                  className="field"
                  value={purpose}
                  onChange={(e) => setPurpose(e.target.value as Purpose)}
                >
                  {Object.entries(purposes).map(([key, label]) => (
                    <option key={key} value={key}>
                      {label}
                    </option>
                  ))}
                </select>
              </label>
              <button
                className="btn-primary"
                disabled={!!busy || !data.configuration.enabled}
              >
                <Search size={15} />
                执行查询
              </button>
            </form>
            {lookup && (
              <div className="mt-5 space-y-3 text-xs" aria-live="polite">
                <p className="break-all font-medium">
                  {lookup.name} · {lookup.query_type} ·{" "}
                  {purposes[lookup.purpose]}
                </p>
                <p>
                  {lookup.response_code} · {lookup.elapsed_ms} ms ·{" "}
                  {lookup.source} · {lookup.protocol.toUpperCase()} /{" "}
                  {lookup.upstream} ·{" "}
                  {lookup.egress ? `代理 ${lookup.egress}` : "DNS 直连"} · 路由{" "}
                  {lookup.route ?? "默认"}
                </p>
                <p className="muted">
                  {lookup.remaining_ttl === null
                    ? "没有可用缓存 TTL，此结果不缓存"
                    : `剩余缓存 TTL ${lookup.remaining_ttl} s`}{" "}
                  · 本机未进行 DNSSEC 验证
                </p>
                {[...lookup.answers, ...lookup.authorities].map((record, i) => (
                  <p
                    key={i}
                    className="break-all rounded-lg bg-console-soft p-3 font-mono leading-6"
                  >
                    {record.name} {record.ttl} IN {record.query_type}{" "}
                    {record.value}
                  </p>
                ))}
                {!lookup.answers.length && (
                  <p className="muted">
                    无 Answer 记录。负缓存仅在有 SOA 证据时保留。
                  </p>
                )}
              </div>
            )}
          </section>
          <section
            id="dns-cache"
            className="panel panel-body"
            aria-label="DNS 缓存"
          >
            <h2 className="font-semibold">缓存与清理</h2>
            <form
              className="mt-4 flex flex-wrap gap-3"
              onSubmit={(e) => {
                e.preventDefault();
                void action("clear", async () => {
                  const result = await postJson<{
                    cleared: number;
                    epoch: number;
                  }>("api/v1/dns/cache/clear", {
                    domain: domain.trim() || null,
                  });
                  if (mounted.current)
                    setInfo(
                      `已清理 ${result.cleared} 条缓存 · 代际 ${result.epoch}，旧在途结果不会回填。`,
                    );
                  await refresh();
                });
              }}
            >
              <label className="form-field flex-1 basis-56">
                清理域名（留空清理全部）
                <input
                  className="field"
                  value={domain}
                  onChange={(e) => setDomain(e.target.value)}
                  maxLength={253}
                />
              </label>
              <button className="btn self-end" disabled={!!busy}>
                <Trash2 size={15} />
                清理缓存
              </button>
            </form>
            <div className="mt-4 divide-y divide-console-border">
              {data.cache.entries.map((entry) => (
                <div
                  key={entry.id}
                  className="flex flex-wrap justify-between gap-2 py-4 text-xs"
                >
                  <div className="min-w-0">
                    <strong className="break-all">
                      {entry.name} · {entry.query_type}
                    </strong>
                    <p className="muted mt-2">
                      {purposes[entry.purpose]} · {entry.upstream} ·{" "}
                      {entry.egress ?? "DNS 直连"} ·{" "}
                      {entry.negative ? "负缓存" : "正缓存"}
                    </p>
                  </div>
                  <span>TTL {entry.remaining_ttl} s</span>
                </div>
              ))}
            </div>
            {!data.cache.entries.length && (
              <p className="muted py-8 text-center text-sm">当前页无缓存</p>
            )}
            <div className="mt-3 flex justify-between">
              <button
                className="btn"
                disabled={!after || !!busy}
                onClick={() => setAfter("")}
              >
                返回首页
              </button>
              <button
                className="btn"
                disabled={!data.cache.next_after || !!busy}
                onClick={() => setAfter(data.cache.next_after ?? "")}
              >
                下一页
              </button>
            </div>
          </section>
          <section
            id="dns-context"
            className="panel panel-body"
            aria-label="DNS 域名上下文"
          >
            <h2 className="font-semibold">域名上下文</h2>
            <form
              className="mt-4 flex flex-wrap items-end gap-3"
              onSubmit={(e) => {
                e.preventDefault();
                void action("context", async () => {
                  const result = await getJson<Context>(
                    `api/v1/dns/context?ip=${encodeURIComponent(ip.trim())}`,
                  );
                  if (mounted.current) setContext(result);
                });
              }}
            >
              <label className="form-field flex-1 basis-56">
                IPv4 / IPv6
                <input
                  className="field"
                  value={ip}
                  onChange={(e) => setIp(e.target.value)}
                  maxLength={45}
                  required
                  placeholder="203.0.113.9"
                />
              </label>
              <button className="btn" disabled={!!busy}>
                查询上下文
              </button>
            </form>
            {context && (
              <div className="mt-4 text-sm">
                <p>
                  {context.domains.length
                    ? context.domains.join("、")
                    : "没有有效的本地 DNS 观测，使用 IP / 未知域名。"}
                </p>
                {context.ambiguous && (
                  <p className="mt-2 text-amber-600">
                    此 IP 对应多个域名，无法据此确定当前连接的目标域名。
                  </p>
                )}
                <ul className="mt-3 space-y-2 text-xs">
                  {(context.mappings ?? []).map((mapping, index) => (
                    <li
                      key={index}
                      className="break-all rounded-lg bg-console-soft p-3"
                    >
                      {mapping.domain} · 上游 {mapping.upstream} · 剩余 TTL{" "}
                      {mapping.remaining_ttl}s · 观察年龄{" "}
                      {mapping.observed_age_ms}ms · 来源：未过期的本地 DNS 应答
                    </li>
                  ))}
                </ul>
                {context.mappings_truncated && (
                  <p className="muted mt-2 text-xs">
                    映射已截断为 64 项，不能据此推断唯一域名。
                  </p>
                )}
              </div>
            )}
            <p className="muted mt-4 text-xs leading-6">
              仅使用未过期的本地 DNS 回答及明确的域名目标。外部 DoH/DoT、ECH
              和未经过本服务的 DNS 不保证可见；DNS
              观测不能证明网关客户端的进程身份。Linux DNS 接管：
              {data.linux_capture_active ? "已启用" : "未启用"}。
            </p>
            {data.linux_capture && (
              <details className="mt-4 rounded-xl bg-console-soft p-4 text-xs leading-6">
                <summary className="cursor-pointer font-medium">
                  Linux DNS 内核证据
                </summary>
                <p className="mt-2 break-all">
                  cgroup：{data.linux_capture.cgroup} · 端口{" "}
                  {data.linux_capture.port} · 已记录 {data.linux_capture.peers}{" "}
                  个 DNS peer
                </p>
                {data.linux_capture.stats.map((stats) => (
                  <p key={stats.family}>
                    {stats.family.toUpperCase()} · 重定向 {stats.redirected} ·
                    地址恢复 {stats.restored} · 错误 {stats.errors}
                  </p>
                ))}
                <p className="break-all">
                  Program ID：{data.linux_capture.program_ids.join("、")}
                </p>
                <p className="break-all font-mono">
                  对象 SHA256：{data.linux_capture.object_sha256}
                </p>
                <p className="muted mt-2">
                  这是所配置 cgroup 的本机应用 DNS 捕获；网关及完整透明 TCP/UDP
                  的状态在 Linux 专用页面显示。
                </p>
              </details>
            )}
          </section>
        </>
      )}
    </div>
  );
}
