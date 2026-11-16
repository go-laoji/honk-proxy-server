import { strategyLabels, type Group } from "../../selection";
import { idChoices, type Catalog } from "../ReferencePicker";
import {
  identifier,
  list,
  number,
  optionalText,
  protocols,
  select,
  text,
  toggle,
  transports,
  type Field,
} from "./ConfigFields";

type RoutingReferences = { groups: Catalog; nodes: Catalog; inbounds: Catalog };
type SelectionReferences = RoutingReferences & { sources: Catalog };

export function groupFields(
  readOnly: boolean,
  group: Group,
  { nodes, sources, groups }: SelectionReferences,
): Field[] {
  return [
    identifier("分组 ID", readOnly),
    select("strategy", "选点策略", strategyLabels),
    toggle("include_all", "包含全部节点"),
    { kind: "references", key: "members", label: "成员节点", catalog: nodes },
    {
      kind: "references",
      key: "providers",
      label: "供应商或订阅",
      catalog: sources,
    },
    {
      kind: "references",
      key: "groups",
      label: "子分组",
      catalog: groups,
      exclude: [group.id],
    },
    list("tags", "筛选标签"),
    {
      kind: "choices",
      key: "protocols",
      label: "协议（不选匹配全部）",
      choices: protocols,
    },
    {
      kind: "reference",
      key: "pinned_node",
      label: "固定节点",
      catalog: nodes,
      required: group.strategy === "fixed" || group.fallback_nodes.length > 0,
    },
    {
      kind: "references",
      key: "fallback_nodes",
      label: "回退节点",
      catalog: nodes,
      ordered: true,
      exclude: group.pinned_node ? [group.pinned_node] : [],
    },
    {
      kind: "section",
      key: "advanced",
      label: "节点权重、评分与会话",
      fields: [
        {
          kind: "map",
          key: "weights",
          label: "节点权重",
          keyLabel: "节点",
          keyCatalog: nodes,
          valueLabel: "权重",
          valueKind: "number",
          min: 1,
          max: 1000,
        },
        {
          kind: "object",
          key: "score",
          label: "综合评分",
          fields: [
            number("latency_weight", "延迟权重", 0, 1000),
            number("lifetime_weight", "剩余有效期权重", 0, 1000),
            number("priority_weight", "运营优先级权重", 0, 1000),
            number("missing_evidence_penalty", "缺失证据扣分", 0, 1000),
            number("latency_ceiling_ms", "延迟上限（毫秒）", 1, 60000),
            number("lifetime_target_secs", "目标剩余有效期（秒）", 1, 86400),
          ],
        },
        {
          kind: "object",
          key: "session",
          label: "会话保持",
          create: () => ({ key: "source_ip", ttl_secs: 300 }),
          fields: [
            select("key", "会话识别方式", {
              source_ip: "来源 IP",
              source_destination: "来源 IP 与目标",
            }),
            number("ttl_secs", "会话有效期（秒）", 1, 86400),
          ],
        },
      ],
    },
  ];
}

export function selectionFields(
  groups: { id: string }[],
  { nodes, sources, inbounds }: SelectionReferences,
): Field[] {
  const groupCatalog = idChoices(groups.map(({ id }) => id));
  return [
    {
      kind: "reference",
      key: "default_group",
      label: "默认分组",
      catalog: groupCatalog,
    },
    number("max_sessions", "会话数量上限", 1, 100000),
    {
      kind: "map",
      key: "inbound_groups",
      label: "入口绑定",
      keyLabel: "入口",
      valueLabel: "分组",
      keyCatalog: inbounds,
      valueCatalog: groupCatalog,
    },
    {
      kind: "map",
      key: "node_tags",
      label: "节点标签",
      keyLabel: "节点",
      keyCatalog: nodes,
      valueLabel: "标签",
      valueKind: "list",
    },
    {
      kind: "map",
      key: "provider_tags",
      label: "来源标签",
      keyLabel: "来源",
      keyCatalog: sources,
      valueLabel: "标签",
      valueKind: "list",
    },
  ];
}

const inboundProtocols = { ...protocols, transparent: "透明接管" };
const action = ({ groups, nodes }: RoutingReferences): Field[] => [
  {
    kind: "select",
    key: "kind",
    label: "流量动作",
    choices: {
      default: "按入口与默认分组选点",
      node: "指定节点",
      group: "指定分组",
      direct: "直连",
      reject: "拒绝",
    },
    resetKeys: ["id"],
  },
  {
    kind: "reference",
    key: "id",
    label: "目标节点",
    catalog: nodes,
    required: true,
    when: (value) => value.kind === "node",
  },
  {
    kind: "reference",
    key: "id",
    label: "目标代理组",
    catalog: groups,
    required: true,
    when: (value) => value.kind === "group",
  },
];

export const routingFields = (references: RoutingReferences): Field[] => [
  toggle("use_dns_observations", "使用 DNS 观测补充域名"),
  {
    kind: "object",
    key: "default_action",
    label: "默认动作",
    fields: action(references),
  },
];

export function ruleFields(
  readOnly: boolean,
  references: RoutingReferences,
): Field[] {
  return [
    identifier("规则 ID", readOnly),
    toggle("enabled", "启用此规则"),
    {
      kind: "object",
      key: "action",
      label: "匹配后的动作",
      fields: action(references),
    },
    {
      kind: "object",
      key: "conditions",
      label: "匹配条件",
      fields: [
        list("domains", "完整域名"),
        list("domain_suffixes", "域名后缀"),
        list("destination_cidrs", "目标 IP 网段（CIDR）"),
        {
          kind: "choices",
          key: "transports",
          label: "传输协议（不选匹配全部）",
          choices: transports,
        },
        {
          kind: "rows",
          key: "destination_ports",
          label: "目标端口范围",
          max: 256,
          create: () => ({ start: 443, end: 443 }),
          help: "只匹配一个端口时，起始和结束填写相同数字。",
          fields: [
            number("start", "起始端口", 1, 65535),
            number("end", "结束端口", 1, 65535),
          ],
        },
        {
          kind: "section",
          key: "advanced",
          label: "入口、来源与本机进程",
          fields: [
            {
              kind: "choices",
              key: "inbound_protocols",
              label: "入口协议（不选匹配全部）",
              choices: inboundProtocols,
            },
            {
              kind: "references",
              key: "inbounds",
              label: "匹配入口",
              catalog: references.inbounds,
            },
            list("source_cidrs", "来源 IP 网段（CIDR）"),
            {
              kind: "object",
              key: "process",
              label: "进程条件",
              create: () => ({}),
              fields: [
                number("pid", "进程 ID", 1, 4294967295, true),
                {
                  kind: "text",
                  key: "command",
                  label: "进程名称",
                  maxLength: 15,
                  nullable: true,
                },
                number(
                  "cgroup_id",
                  "cgroup ID",
                  1,
                  Number.MAX_SAFE_INTEGER,
                  true,
                ),
              ],
            },
          ],
        },
      ],
    },
  ];
}

export const simulationFields = (inbounds: Catalog): Field[] => [
  text("destination_host", "目标域名或 IP", true),
  number("destination_port", "目标端口", 1, 65535),
  select("transport", "传输协议", transports),
  { kind: "reference", key: "inbound", label: "入口", catalog: inbounds },
  {
    kind: "section",
    key: "advanced",
    label: "来源、DNS 观测与进程",
    fields: [
      optionalText("source_ip", "来源 IP"),
      number("source_port", "来源端口", 1, 65535, true),
      select("inbound_protocol", "入口协议", inboundProtocols, true),
      list("observed_domains", "DNS 观测域名"),
      select("origin", "流量来源", {
        explicit: "显式代理",
        linux_local: "Linux 本机",
        gateway: "网关",
      }),
      {
        kind: "object",
        key: "process",
        label: "进程身份",
        create: () => ({ pid: null, command: "", cgroup_id: null }),
        fields: [
          number("pid", "进程 ID", 1, 4294967295),
          text("command", "进程名称", true),
          number("cgroup_id", "cgroup ID", 1, Number.MAX_SAFE_INTEGER),
        ],
      },
    ],
  },
];
