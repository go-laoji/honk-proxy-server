import type { ProbeResult } from "./types";
export type NodeControl = {
  node_id: string;
  state: "enabled" | "disabled" | "draining" | "deleted";
  version: number;
  updated_at: number;
};
export type NodeView = {
  id: string;
  kind: string;
  provider_id: string | null;
  lease_id: number | null;
  batch_id: number | null;
  protocol: string;
  host: string;
  port: number;
  configuration_enabled: boolean;
  has_credentials: boolean;
  received_at: number | null;
  expires_at_unix: number | null;
  remaining_secs: number | null;
  expiry_basis: string | null;
  region: string | null;
  lease_state: string;
  operation: NodeControl;
  published: boolean;
  eligible: boolean;
  admission_reasons: string[];
  tcp_supported: boolean;
  udp_supported: boolean;
  health_state: string;
  health_results: ProbeResult[];
  groups: string[];
  tags: string[];
  active_connections: number;
};
export type NodeSummary = {
  inventory: number;
  published: number;
  eligible: number;
  near_expiry: number;
  expired: number;
  restricted: number;
  source_unavailable: number;
  active_connections: number;
};
export type NodesOverview = {
  running_revision: number;
  evaluated_at_unix: number;
  summary: NodeSummary;
  inventory_truncated: boolean;
  providers: string[];
  groups: string[];
  sources: ProviderNodeSummary[];
  filtered_total: number;
  offset: number;
  next_offset: number | null;
  nodes: NodeView[];
};
export type ProviderNodeSummary = {
  id: string;
  enabled: boolean;
  refill_enabled: boolean;
  minimum: number;
  target: number;
  maximum: number;
  inventory: number;
  eligible: number;
  near_expiry: number;
  expired: number;
  latest_task_id: number | null;
  latest_task_state: string | null;
  latest_error_code: string | null;
  latest_task_at: number | null;
};
export type ImportRequest = {
  document: string;
  format: string;
  default_protocol: string;
  expected_revision: number;
  preview_checksum: string | null;
  selected_rows: number[];
};
export type ImportPreview = {
  running_revision: number;
  checksum: string;
  valid_count: number;
  invalid_count: number;
  selected_count: number;
  applicable: boolean;
  rows: {
    row: number;
    id: string | null;
    protocol: string | null;
    host: string | null;
    port: number | null;
    has_credentials: boolean;
    valid: boolean;
    error_code: string | null;
  }[];
};
export const nodeStates: Record<string, string> = {
  active: "租约有效",
  subscription_snapshot: "标准订阅快照（有效期未知）",
  static: "静态配置",
  expired: "已到期",
  revoked: "快照已撤销",
  clock_rollback: "时钟回退已退休",
  source_deleted: "来源已删除",
  source_disabled: "来源已停用",
  source_changed: "来源配置已变更",
  enabled: "无人工限制",
  disabled: "已禁用",
  draining: "排空中",
  deleted: "已软删除",
  healthy: "健康通过",
  unhealthy: "检测失败",
  unknown: "尚无有效健康证据",
  checking_disabled: "已关闭健康检查",
};
export const nodeReasons: Record<string, string> = {
  configuration_disabled: "配置中未启用",
  manually_disabled: "人工禁用",
  draining: "停止新连接并排空",
  manually_deleted: "已软删除，可恢复",
  expired: "已到期",
  insufficient_lifetime: "剩余寿命不足准入窗口",
  health_unavailable: "缺少新鲜健康证据",
  not_published: "未进入当前池（可能被去重、水位或来源策略排除）",
  source_deleted: "来源已删除",
  source_disabled: "来源已停用",
  source_changed: "来源指纹已变化",
  revoked: "快照已撤销",
  clock_rollback: "时钟回退已退休",
};
export const nodeAddress = (n: {
  protocol: string;
  host: string;
  port: number;
}) =>
  `${n.protocol}://${n.host.includes(":") ? `[${n.host}]` : n.host}:${n.port}`;
