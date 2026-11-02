export type SelectionDecision = {
  group: string | null;
  strategy: string;
  selected: string | null;
  reason: string;
  session_reused: boolean;
  session_remaining_secs: number | null;
  total: number;
  eligible: number;
  sessions: number;
  next_after: string | null;
  candidates: {
    node_id: string;
    provider_id: string | null;
    protocol: string;
    tags: string[];
    eligible: boolean;
    reasons: string[];
    remaining_secs: number | null;
    latency_ms: number | null;
    weight: number;
  }[];
};
export const strategyLabels: Record<string, string> = {
  fixed: "固定",
  failover: "故障切换",
  latency: "延迟优选",
  round_robin: "轮询",
  weighted: "加权轮询",
};
export const candidateReasons: Record<string, string> = {
  manually_disabled: "人工禁用",
  draining: "正在排空",
  manually_deleted: "已软删除",
  different_explicit_node: "不是规则指定的节点",
  not_member: "不属于分组",
  tag_filter: "标签不匹配",
  protocol_filter: "代理协议不匹配",
  outside_pin_fallback: "超出固定与回退范围",
  tcp_only: "仅支持 TCP",
  no_fresh_latency: "无新鲜延迟证据",
  disabled: "节点已停用",
  expired: "租约已到期",
  insufficient_lifetime: "剩余寿命不足",
  source_reconfigured: "来源配置已变化",
  health_unavailable: "无合格健康证据",
  missing_or_retired: "节点缺失或已退休",
  unknown_group: "分组不存在",
  no_eligible_candidate: "没有合格候选",
  pinned: "手动固定",
  session: "复用会话",
  explicit_fallback: "显式回退",
  strategy: "按策略选择",
};
