import type { ProviderConfig } from "../../types";
import {
  identifier,
  number,
  optionalText,
  protocols,
  select,
  text,
  toggle,
  type Field,
} from "./ConfigFields";

const kinds = {
  kuaidaili: "快代理",
  yun_free: "云代理免费列表",
  json: "通用 JSON API",
  text: "纯文本列表",
};
const json = {
  list: "/data/proxies",
  host: "/ip",
  port: "/port",
  ttl: "/ttl",
  expires_at: null,
  expiry_milliseconds: false,
};
const refillDefaults = {
  enabled: false,
  mode: "on_demand",
  min_nodes: 5,
  target_nodes: 10,
  max_nodes: 20,
  batch_size: 5,
  retire_before_secs: 60,
  forecast_secs: 30,
  idle_timeout_secs: 300,
  daily_request_budget: 10,
  daily_node_budget: 100,
  backoff_initial_secs: 5,
  backoff_max_secs: 900,
  failure_limit: 5,
};

export function providerDraft(
  config?: Partial<ProviderConfig>,
): ProviderConfig {
  return {
    id: "",
    name: "",
    enabled: false,
    kind: "json",
    endpoint: "",
    protocol: "http",
    count: 10,
    ttl_secs: null,
    min_interval_secs: 60,
    query_valid_time: false,
    method: "GET",
    auth: { type: "none" },
    mode: "append_leases",
    full_snapshot: false,
    minimum_snapshot_percent: 50,
    min_remaining_secs: 30,
    timeout_ms: 10000,
    max_response_bytes: 2097152,
    json,
    ...config,
    refill: config?.refill ?? refillDefaults,
  };
}

export function providerPayload(config: ProviderConfig): ProviderConfig {
  const auth = config.auth ?? { type: "none" };
  return {
    ...config,
    query_valid_time: config.kind === "kuaidaili" && config.query_valid_time,
    auth:
      auth.type === "token"
        ? { type: auth.type, secret_id: auth.secret_id, token: auth.token }
        : auth.type === "hmac_sha1"
          ? {
              type: auth.type,
              secret_id: auth.secret_id,
              secret_key: auth.secret_key,
            }
          : { type: "none" },
  };
}

export function providerFields(
  readOnly: boolean,
  draft: ProviderConfig,
): Field[] {
  const needsTtl =
    draft.kind === "text" ||
    draft.kind === "yun_free" ||
    (draft.kind === "kuaidaili" && !draft.query_valid_time);
  return [
    identifier("来源 ID", readOnly),
    { kind: "text", key: "name", label: "显示名称", maxLength: 128 },
    select("kind", "供应商类型", kinds),
    select("protocol", "节点代理协议", protocols),
    { kind: "url", key: "endpoint", label: "提取 API 地址", required: true },
    toggle("enabled", "启用供应商"),
    number("count", "单次提取数量", 1, 1000),
    number("ttl_secs", "默认有效期（秒）", 1, 31536000, !needsTtl),
    {
      kind: "boolean",
      key: "query_valid_time",
      label: "向快代理查询剩余有效期",
      when: (draft) => draft.kind === "kuaidaili",
    },
    {
      kind: "section",
      key: "request",
      label: "请求参数与鉴权",
      fields: [
        select("method", "请求方法", { GET: "GET", POST: "POST" }),
        optionalText("count_parameter", "数量参数名"),
        {
          kind: "map",
          key: "params",
          label: "请求参数",
          keyLabel: "参数名",
          valueLabel: "参数值",
          valueKind: "password",
        },
        {
          kind: "map",
          key: "headers",
          label: "请求头",
          keyLabel: "请求头名称",
          valueLabel: "请求头值",
          valueKind: "password",
        },
        {
          kind: "object",
          key: "auth",
          label: "API 鉴权",
          fields: [
            select("type", "鉴权方式", {
              none: "无鉴权",
              token: "Token",
              hmac_sha1: "HMAC-SHA1",
            }),
            {
              kind: "password",
              key: "secret_id",
              label: "Secret ID",
              required: true,
              when: (auth) => auth.type !== "none",
            },
            {
              kind: "password",
              key: "token",
              label: "Token",
              required: true,
              when: (auth) => auth.type === "token",
            },
            {
              kind: "password",
              key: "secret_key",
              label: "Secret Key",
              required: true,
              when: (auth) => auth.type === "hmac_sha1",
            },
          ],
        },
        {
          kind: "password",
          key: "fetch_proxy",
          label: "提取请求的代理 URL",
          nullable: true,
        },
        {
          kind: "object",
          key: "upstream_credentials",
          label: "节点代理凭据",
          create: () => ({ username: "", password: "" }),
          fields: [
            {
              kind: "password",
              key: "username",
              label: "节点代理用户名",
              required: true,
              maxLength: 255,
            },
            {
              kind: "password",
              key: "password",
              label: "节点代理密码",
              maxLength: 255,
            },
          ],
        },
      ],
    },
    {
      kind: "object",
      key: "json",
      label: "JSON 响应字段映射",
      collapsible: true,
      when: (draft) => draft.kind === "json",
      help: "填写以 / 开头的 JSON Pointer，例如 /data/proxies。",
      fields: [
        text("list", "节点列表路径", true),
        text("host", "节点主机字段路径", true),
        text("port", "节点端口字段路径", true),
        optionalText("ttl", "剩余有效期字段路径"),
        optionalText("expires_at", "到期时间字段路径"),
        toggle("expiry_milliseconds", "到期时间使用毫秒"),
      ],
    },
    {
      kind: "section",
      key: "lease",
      label: "租约与请求限制",
      fields: [
        select("mode", "更新方式", {
          append_leases: "追加租约",
          replace_snapshot: "替换快照",
        }),
        toggle("full_snapshot", "供应商返回完整快照"),
        number("minimum_snapshot_percent", "快照最小保留比例（%）", 0, 100),
        number("min_remaining_secs", "节点最少剩余时长（秒）", 0, 86400),
        number("min_interval_secs", "最短提取间隔（秒）", 1, 86400),
        number("timeout_ms", "请求超时（毫秒）", 100, 120000),
        number("max_response_bytes", "响应体上限（字节）", 1024, 4194304),
      ],
    },
    {
      kind: "object",
      key: "refill",
      label: "自动补池",
      collapsible: true,
      help: "通用 API 启用补池时，请在请求参数中填写供应商支持的数量参数名。",
      fields: [
        toggle("enabled", "启用自动补池"),
        select("mode", "补池模式", {
          on_demand: "按需补池",
          maintain: "持续维持水位",
        }),
        number("min_nodes", "最低节点数", 1, 100000),
        number("target_nodes", "目标节点数", 1, 100000),
        number("max_nodes", "最大节点数", 1, 100000),
        number("batch_size", "每批补充节点数", 1, 1000),
        number("retire_before_secs", "提前退出补池水位（秒）", 0, 86400),
        number("forecast_secs", "预估使用窗口（秒）", 0, 86400),
        number("idle_timeout_secs", "空闲超时（秒）", 1, 86400),
        number("daily_request_budget", "每日请求上限", 1, 1000000),
        number("daily_node_budget", "每日节点上限", 1, 1000000000),
        number("backoff_initial_secs", "首次退避间隔（秒）", 1, 86400),
        number("backoff_max_secs", "最长退避间隔（秒）", 1, 86400),
        number("failure_limit", "暂停前失败次数", 1, 100),
      ],
    },
  ];
}

export function subscriptionFields(readOnly: boolean): Field[] {
  return [
    identifier("订阅 ID", readOnly, 24),
    { kind: "text", key: "name", label: "显示名称", maxLength: 128 },
    {
      kind: "password",
      key: "url",
      label: "订阅 URL",
      required: true,
      maxLength: 4096,
    },
    select("format", "订阅格式", {
      uri: "代理链接列表（URI）",
      clash: "Clash YAML",
    }),
    toggle("enabled", "启用订阅"),
    {
      kind: "section",
      key: "advanced",
      label: "请求与节点限制",
      fields: [
        number("timeout_ms", "请求超时（毫秒）", 100, 120000),
        number("max_nodes", "快照节点上限", 1, 10000),
      ],
    },
  ];
}
