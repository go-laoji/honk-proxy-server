export type Action =
  | { kind: "default" | "direct" | "reject" }
  | { kind: "node" | "group"; id: string };
export type Conditions = {
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
export type Rule = {
  id: string;
  enabled: boolean;
  conditions: Conditions;
  action: Action;
};

export type QuickRuleSeed = {
  target?: string;
  group?: string;
  port?: number;
  transport?: "tcp" | "udp";
};

export function quickRuleHref(seed: QuickRuleSeed) {
  const params = new URLSearchParams({ create: "group-rule" });
  if (seed.target) params.set("target", seed.target);
  if (seed.group) params.set("group", seed.group);
  if (seed.port) params.set("port", String(seed.port));
  if (seed.transport) params.set("transport", seed.transport);
  return `./rules.html?${params}`;
}

export function readQuickRuleSeed(
  params: URLSearchParams,
): QuickRuleSeed | null {
  if (params.get("create") !== "group-rule") return null;
  const port = Number(params.get("port"));
  const transport = params.get("transport");
  return {
    target: params.get("target") ?? undefined,
    group: params.get("group") ?? undefined,
    port:
      Number.isInteger(port) && port >= 1 && port <= 65535 ? port : undefined,
    transport:
      transport === "tcp" || transport === "udp" ? transport : undefined,
  };
}
