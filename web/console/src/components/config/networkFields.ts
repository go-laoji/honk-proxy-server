import {
  identifier,
  list,
  number,
  optionalText,
  select,
  text,
  toggle,
  transports,
  type Field,
} from "./ConfigFields";

const probe: Field[] = [
  select("transport", "传输协议", transports),
  select("family", "IP 版本", { ipv4: "IPv4", ipv6: "IPv6" }),
  { kind: "url", key: "url", label: "探测 URL", required: true },
  number("expected_status", "预期 HTTP 状态码（可选）", 100, 599, true),
  toggle("enabled", "启用此探测"),
  toggle("capture_exit_ip", "从响应读取出口 IP"),
  {
    kind: "section",
    key: "tls",
    label: "TLS 设置",
    fields: [
      optionalText("tls_server_name", "TLS 服务名称"),
      optionalText("ca_certificate", "CA 证书路径"),
    ],
  },
];

export const healthFields: Field[] = [
  toggle("enabled", "启用健康检测"),
  number("interval_secs", "检测间隔（秒）", 1, 86400),
  number("timeout_ms", "单次超时（毫秒）", 100, 60000),
  number("max_parallel", "最大并行检测数", 1, 64),
  {
    kind: "rows",
    key: "profiles",
    label: "探测档案",
    fields: probe,
    max: 4,
    help: "同一种传输协议与 IP 版本只能配置一项。当前版本可保留停用的 UDP 档案。",
    create: () => ({
      transport: "tcp",
      family: "ipv4",
      enabled: true,
      url: "https://www.baidu.com/",
    }),
  },
  {
    kind: "section",
    key: "advanced",
    label: "队列、重试与资源上限",
    fields: [
      number("queue_capacity", "队列容量", 16, 10000),
      number("daily_probe_budget", "每日检测上限", 1, 1000000),
      number("max_staleness_secs", "结果有效期（秒）", 1, 86400),
      number("min_probe_margin_secs", "节点最少剩余时长（秒）", 0, 86400),
      number("retry_initial_secs", "首次重试间隔（秒）", 1, 86400),
      number("retry_max_secs", "最长重试间隔（秒）", 1, 86400),
      number("traffic_failure_threshold", "连接失败计数阈值", 1, 100),
      number("max_header_bytes", "响应头大小上限（字节）", 8192, 65536),
    ],
  },
];

const upstream: Field[] = [
  identifier("上游 ID"),
  select("protocol", "解析协议", {
    system: "系统解析",
    udp: "UDP",
    tcp: "TCP",
    tls: "DNS over TLS",
    https: "DNS over HTTPS",
  }),
  {
    ...list("addresses", "上游 IP 与端口"),
    help: "每行一个地址，例如 1.1.1.1:53 或 [2606:4700:4700::1111]:53。系统解析留空。",
  },
  {
    kind: "section",
    key: "connection",
    label: "加密与代理出口",
    fields: [
      optionalText("server_name", "TLS 服务名称"),
      optionalText("https_path", "DoH 请求路径"),
      optionalText("ca_certificate", "CA 证书路径"),
      optionalText("proxy_id", "静态代理节点 ID"),
      list("proxy_bootstrap_ips", "代理引导 IP"),
    ],
  },
];

export function dnsFields(
  upstreams: { id: string }[],
  purposes: Record<string, string>,
): Field[] {
  const choices = Object.fromEntries(
    upstreams.filter(({ id }) => id).map(({ id }) => [id, id]),
  );
  return [
    toggle("enabled", "启用 DNS 策略"),
    select("default_upstream", "默认上游", choices),
    {
      kind: "rows",
      key: "upstreams",
      label: "DNS 上游",
      fields: upstream,
      max: 64,
      create: () => ({
        id: "",
        protocol: "udp",
        addresses: [],
        proxy_bootstrap_ips: [],
      }),
    },
    {
      kind: "rows",
      key: "routes",
      label: "解析路由",
      ordered: true,
      max: 1024,
      help: "按排列顺序匹配，第一条命中的路由生效。",
      create: () => ({
        id: "",
        upstream: "",
        suffix: false,
        query_types: [],
        purposes: [],
      }),
      fields: [
        identifier("路由 ID"),
        select("upstream", "目标上游", choices),
        optionalText("domain", "域名（留空匹配所有域名）"),
        toggle("suffix", "同时匹配子域"),
        list("query_types", "查询类型（留空匹配全部）"),
        {
          kind: "choices",
          key: "purposes",
          label: "解析用途（不选匹配全部）",
          choices: purposes,
        },
      ],
    },
    {
      kind: "section",
      key: "limits",
      label: "监听、缓存与 Linux 捕获",
      fields: [
        optionalText("listen", "DNS 监听地址"),
        number("timeout_ms", "解析超时（毫秒）", 100, 60000),
        number("max_parallel", "最大并行查询数", 1, 256),
        number("cache_entries", "缓存条目上限", 0, 100000),
        number("cache_bytes", "缓存容量（字节）", 0, 268435456),
        {
          kind: "object",
          key: "capture",
          label: "Linux DNS 捕获",
          create: () => ({ object: "", cgroup: "", port: 1053 }),
          fields: [
            text("object", "eBPF 对象路径", true),
            text("cgroup", "cgroup v2 绝对路径", true),
            number("port", "捕获端口", 1, 65535),
          ],
        },
      ],
    },
  ];
}
