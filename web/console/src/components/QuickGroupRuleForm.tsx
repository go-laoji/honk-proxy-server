import { useState, type FormEvent } from "react";
import type { Conditions, QuickRuleSeed } from "../routing";

const matches = {
  domains: "完整域名",
  domain_suffixes: "域名后缀",
  destination_cidrs: "目标 IP 或 CIDR",
};
type Match = keyof typeof matches;
const unwrapIp = (target: string) =>
  target.replace(/^\[([^\]]+)\](?=\/|$)/, "$1");
const isIp = (value: string) =>
  /^(?:\d{1,3}\.){3}\d{1,3}$/.test(value) ||
  (value.includes(":") && /^[\da-f:.]+$/i.test(value));

export type QuickGroupRule = {
  group: string;
  conditions: Conditions;
  first: boolean;
  example: { host: string; port: number; transport: "tcp" | "udp" };
};

export default function QuickGroupRuleForm({
  seed,
  groups,
  disabled,
  busy,
  onSave,
  onCancel,
}: {
  seed: QuickRuleSeed;
  groups: string[];
  disabled: boolean;
  busy: boolean;
  onSave: (rule: QuickGroupRule) => void;
  onCancel: () => void;
}) {
  const [match, setMatch] = useState<Match>(() =>
    isIp(unwrapIp(seed.target ?? "").split("/")[0])
      ? "destination_cidrs"
      : "domains",
  );
  const [target, setTarget] = useState(seed.target ?? "");
  const [group, setGroup] = useState(seed.group ?? "");
  const [port, setPort] = useState(seed.port ? String(seed.port) : "");
  const [transport, setTransport] = useState<"tcp" | "udp" | "">(
    seed.transport ?? "",
  );
  const [first, setFirst] = useState(false);
  const [error, setError] = useState("");
  const selectedGroup = groups.includes(group) ? group : "";

  function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (disabled || !selectedGroup) return;
    let value = target.trim();
    if (match === "domain_suffixes") value = value.replace(/^\*?\./, "");
    if (match === "destination_cidrs") value = unwrapIp(value);
    const host = value.split("/")[0];
    if (
      !value ||
      (match === "destination_cidrs"
        ? !isIp(host)
        : /[\s/:*]/.test(value) || isIp(value))
    ) {
      setError(
        match === "destination_cidrs"
          ? "请填写 IP 地址或 CIDR 网段。"
          : "请填写域名，不含协议和端口。",
      );
      return;
    }
    if (match === "destination_cidrs" && !value.includes("/")) {
      value = `${value}/${value.includes(":") ? 128 : 32}`;
    }
    const conditions: Conditions = { [match]: [value] };
    if (port)
      conditions.destination_ports = [
        { start: Number(port), end: Number(port) },
      ];
    if (transport) conditions.transports = [transport];
    setError("");
    onSave({
      group: selectedGroup,
      conditions,
      first,
      example: {
        host,
        port: port ? Number(port) : 443,
        transport: transport || "tcp",
      },
    });
  }

  return (
    <section className="panel panel-body space-y-3" aria-label="按目标分流">
      <h2 className="font-semibold">按目标分流</h2>
      <form className="space-y-3" aria-label="快速分流规则" onSubmit={save}>
        <fieldset
          className="grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-3"
          disabled={disabled}
        >
          <label className="form-field">
            匹配方式
            <select
              className="field"
              value={match}
              onChange={(event) => {
                setError("");
                setMatch(event.target.value as Match);
              }}
            >
              {Object.entries(matches).map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label className="form-field">
            匹配目标
            <input
              className="field"
              required
              maxLength={253}
              spellCheck={false}
              value={target}
              placeholder={
                match === "destination_cidrs"
                  ? "例如 192.0.2.0/24"
                  : "例如 example.com"
              }
              onChange={(event) => {
                setError("");
                setTarget(event.target.value);
              }}
            />
          </label>
          <label className="form-field">
            目标代理组
            <select
              className="field"
              required
              value={selectedGroup}
              onChange={(event) => setGroup(event.target.value)}
            >
              <option value="" disabled>
                选择代理组
              </option>
              {groups.map((id) => (
                <option key={id} value={id}>
                  {id}
                </option>
              ))}
            </select>
          </label>
          <label className="form-field">
            目标端口
            <input
              className="field"
              type="number"
              step={1}
              min={1}
              max={65535}
              placeholder="不限"
              value={port}
              onChange={(event) => setPort(event.target.value)}
            />
          </label>
          <label className="form-field">
            传输协议
            <select
              className="field"
              value={transport}
              onChange={(event) =>
                setTransport(event.target.value as "tcp" | "udp" | "")
              }
            >
              <option value="">不限</option>
              <option value="tcp">TCP</option>
              <option value="udp">UDP</option>
            </select>
          </label>
          <label className="form-field">
            规则位置
            <select
              className="field"
              value={first ? "first" : "last"}
              onChange={(event) => setFirst(event.target.value === "first")}
            >
              <option value="last">追加到末尾</option>
              <option value="first">放在首位</option>
            </select>
          </label>
        </fieldset>
        {groups.length === 0 ? (
          <p className="muted text-sm">
            请先在
            <a className="text-console-accent-ink" href="./groups.html">
              分组页
            </a>
            创建代理组。
          </p>
        ) : group && !selectedGroup ? (
          <p className="text-sm text-console-warning">
            所选分组已不存在，请重新选择。
          </p>
        ) : null}
        {error && (
          <p role="alert" className="text-sm text-console-danger">
            {error}
          </p>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="submit"
            className="btn-primary"
            disabled={disabled || !selectedGroup}
          >
            保存分流候选
          </button>
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={onCancel}
          >
            取消
          </button>
          <p className="muted text-xs">
            按列表首条命中。保存后可先模拟，再激活。
          </p>
        </div>
      </form>
    </section>
  );
}
