import type { ProxyStatus } from "./types";

export type ConnectionStatistics = {
  epoch: string;
  started_at_unix_ms: number;
  elapsed_ms: number;
  active: number;
  active_tracked: number;
  active_untracked: number;
  active_record_limit: number;
  recent_record_limit: number;
  started: number;
  opened: number;
  closed: number;
  failed: number;
  rejected: number;
  disconnected: number;
  untracked_total: number;
  bytes_up: number;
  bytes_down: number;
  errors: Record<string, number>;
};
export type ConnectionInfo = {
  id: string;
  epoch: string;
  created_at_unix_ms: number;
  ended_at_unix_ms: number | null;
  elapsed_ms: number;
  context: {
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
  observed_domains_truncated: boolean;
  routing: {
    configuration_revision: number;
    evaluated_at_unix: number;
    rule_id: string | null;
    action: { kind: string; id?: string };
    domain: string | null;
    domain_source: string;
  };
  choice: {
    node_id: string;
    provider_id: string | null;
    upstream_host: string;
    upstream_port: number;
    upstream_protocol: string;
    expires_at_unix: number | null;
    remaining_secs_at_admission: number | null;
    minimum_remaining_secs: number;
  } | null;
  selection: {
    group: string | null;
    strategy: string;
    reason: string;
    selected: string | null;
    session_reused: boolean;
  } | null;
  state: string;
  phase: string;
  bytes_up: number;
  bytes_down: number;
  http_status: number | null;
  error_code: string | null;
  close_reason: string | null;
  disconnect_requested: boolean;
};
export type ConnectionsOverview = {
  running_revision: number;
  epoch: string;
  snapshot: string;
  snapshot_created_at_unix_ms: number;
  snapshot_expires_at_unix_ms: number;
  statistics: ConnectionStatistics;
  filtered_total: number;
  offset: number;
  next_offset: number | null;
  records: ConnectionInfo[];
  inbounds: string[];
  groups: string[];
  providers: string[];
};
export type ConnectionSummary = {
  running_revision: number;
  evaluated_at_unix_ms: number;
  statistics: ConnectionStatistics;
  proxy: ProxyStatus;
  attention: {
    kind: string;
    code: string;
    severity: string;
    resource_id: string | null;
    observed_at_unix_ms: number;
    href: string;
  }[];
};
export type DisconnectPreview = {
  token: string;
  epoch: string;
  created_at_unix_ms: number;
  expires_at_unix_ms: number;
  active_untracked_in_instance: number;
  targets: {
    id: string;
    source_ip: string | null;
    source_port: number | null;
    inbound: string | null;
    destination_host: string;
    destination_port: number;
    node_id: string | null;
    configuration_revision: number;
  }[];
};
export type DisconnectResult = {
  epoch: string;
  token: string | null;
  requested: number;
  already_ended: number;
  matched: number;
  requested_at_unix_ms: number;
};
export const connectionStates: Record<string, string> = {
  dialing: "正在拨号",
  open: "转发中",
  disconnecting: "正在断开",
  closed: "已关闭",
  failed: "失败",
  rejected: "规则拒绝",
  disconnected: "已主动断开",
};
export const connectionErrors: Record<string, string> = {
  rule_rejected: "规则拒绝",
  no_eligible_upstream: "无可用出口",
  node_admission_changed: "节点人工准入已变化",
  node_no_longer_eligible: "节点健康或寿命已变化",
  endpoint_dns_failed: "出口域名解析失败",
  endpoint_connect_failed: "出口连接失败",
  upstream_tls_failed: "上游 TLS 失败",
  upstream_authentication_failed: "上游认证失败",
  target_connect_failed: "目标连接失败",
  connect_timeout: "拨号超时",
  upstream_target_policy: "上游拒绝目标",
  proxy_handshake_failed: "代理协商失败",
  local_resource_exhausted: "本地资源不足",
  upstream_http_handshake_failed: "HTTP 交换初始化失败",
  upstream_response_failed: "目标响应失败",
  request_body_failed: "请求流异常",
  response_body_failed: "响应流异常",
  stream_read_failed: "读流异常",
  stream_write_failed: "写流异常",
  stream_flush_failed: "刷新流异常",
  stream_shutdown_failed: "半关闭异常",
  target_rejected: "目标返回 403 / 429",
  connection_disconnected: "连接已断开",
  unsupported_transport: "当前传输不支持",
};
export const endpointLabel = (host: string, port: number) =>
  `${host.includes(":") ? `[${host}]` : host}:${port}`;
export function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index++;
  }
  return `${index === 0 ? Math.floor(value) : value.toFixed(1)} ${units[index]}`;
}
export type Rates = { up: number; down: number; interval: number };
export function sampleRates(
  previous: ConnectionStatistics | null,
  current: ConnectionStatistics,
): Rates | null {
  if (!previous || previous.epoch !== current.epoch) return null;
  const interval = current.elapsed_ms - previous.elapsed_ms;
  const up = current.bytes_up - previous.bytes_up,
    down = current.bytes_down - previous.bytes_down;
  return interval >= 200 && up >= 0 && down >= 0
    ? { up: (up * 1000) / interval, down: (down * 1000) / interval, interval }
    : null;
}
