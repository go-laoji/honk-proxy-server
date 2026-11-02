export type TaskInfo = {
  id: number;
  version: number;
  kind: string;
  source_ref: string | null;
  trigger: string;
  state: string;
  phase: string;
  terminal: boolean;
  created_at_unix_ms: number;
  started_at_unix_ms: number | null;
  updated_at_unix_ms: number;
  finished_at_unix_ms: number | null;
  configuration_revision: number | null;
  configuration_fingerprint: string | null;
  resource_kind: string | null;
  resource_id: string | null;
  request_id: string | null;
  total: number;
  completed: number;
  succeeded: number;
  failed: number;
  skipped: number;
  removed: number;
  error_code: string | null;
  result_ref: string | null;
  result: unknown;
};
export type TasksOverview = {
  epoch: string;
  observed_at_unix_ms: number;
  records: TaskInfo[];
  next_before: number | null;
  terminal_retention_limit: number;
};
export type ServiceEvent = {
  sequence: number;
  recorded_epoch: string;
  occurred_at_unix_ms: number;
  level: string;
  module: string;
  code: string;
  task_id: number | null;
  task_version: number | null;
  request_id: string | null;
  resource_kind: string | null;
  resource_id: string | null;
  node_id: string | null;
  provider_id: string | null;
  peer: string | null;
  action: string | null;
  outcome: string | null;
  configuration_revision: number | null;
};
export type EventsOverview = {
  epoch: string;
  observed_at_unix_ms: number;
  records: ServiceEvent[];
  cursor: string;
  oldest_available_sequence: number;
  head_sequence: number;
  has_more: boolean;
  reset_reason: string | null;
  retention_limit: number;
};
export type EventQuery = {
  cursor?: string;
  level?: string;
  module?: string;
  task_id?: number;
  request_id?: string;
  node_id?: string;
  provider_id?: string;
  after_unix_ms?: number;
  limit?: number;
};
export type EventFrame =
  | {
      kind: "snapshot" | "events" | "checkpoint" | "reset";
      data: EventsOverview;
    }
  | { kind: "unavailable"; data: { code: string } };
export type FeedState = {
  state: "connecting" | "live" | "reconnecting" | "paused" | "hidden" | "error";
  message?: string;
  attempt?: number;
};
export const taskKinds: Record<string, string> = {
  provider: "供应商提取",
  health: "健康检测",
  activation: "配置激活",
  configuration_prepare: "配置候选预检",
  configuration_validate: "配置校验",
  dns_diagnostic: "DNS 诊断",
  provider_diagnostic: "供应商诊断",
  rules_diagnostic: "规则模拟",
  linux_capture_control: "透明代理启停与恢复",
  group_diagnostic: "分组模拟",
};
export const taskStates: Record<string, string> = {
  queued: "排队中",
  running: "进行中",
  completed: "已完成",
  failed: "失败",
  outcome_unknown: "结果不确定",
  discarded: "已丢弃",
  interrupted: "已中断",
  pending_restart: "等待重启",
};
export const eventModules: Record<string, string> = {
  provider: "供应商",
  health: "健康检测",
  configuration: "配置",
  diagnostic: "诊断",
  management: "管理操作",
  runtime: "运行状态",
  linux: "Linux",
  dns: "DNS",
  routing: "路由",
};
export const eventLevels: Record<string, string> = {
  info: "信息",
  warn: "提醒",
  error: "异常",
};
export const feedStates: Record<FeedState["state"], string> = {
  connecting: "正在连接",
  live: "实时订阅中",
  reconnecting: "正在重连",
  paused: "已暂停",
  hidden: "页面隐藏，已暂停",
  error: "订阅已停止",
};
export const resetReasons: Record<string, string> = {
  instance_changed: "服务实例已变化，已清空旧列表并补新实例快照。",
  cursor_expired:
    "游标已超出事件保留窗口，已补最近快照；期间部分事件无法恢复。",
  cursor_ahead: "游标超出现有日志，已重新读取当前快照。",
};
export const taskErrors: Record<string, string> = {
  configuration_invalid: "配置未通过校验",
  configuration_changed: "配置版本已变化",
  configuration_not_found: "配置版本不存在",
  runtime_stopping: "服务正在停止",
  process_interrupted: "进程中断，不能视为已回滚",
  background_operation_failed: "后台工作失败",
  routing_node_unavailable: "规则引用的节点不可用",
  dns_diagnostic_failed: "DNS 诊断失败",
};
export const dateMs = (time: number | null) =>
  time === null ? "未记录" : new Date(time).toLocaleString("zh-CN");
export function queryString(
  query: Record<string, string | number | undefined>,
): string {
  const params = new URLSearchParams();
  Object.entries(query)
    .sort(([a], [b]) => a.localeCompare(b))
    .forEach(([key, value]) => {
      if (value !== undefined && value !== "") params.set(key, String(value));
    });
  return params.toString();
}
export function sourceLink(
  task: TaskInfo,
  epoch: string,
): { label: string; href: string } | null {
  if (task.kind === "linux_capture_control")
    return { label: "透明代理运行控制", href: "./linux.html" };
  const source = task.source_ref?.split(":");
  if (source?.[0] === "provider" && task.resource_id)
    return {
      label: "供应商提取详情",
      href: `./provider-detail.html?id=${encodeURIComponent(task.resource_id)}`,
    };
  if (source?.[0] === "health")
    return {
      label: "健康任务详情",
      href: `./health.html?task=${encodeURIComponent(source[1])}&epoch=${encodeURIComponent(epoch)}`,
    };
  if (task.kind === "rules_diagnostic" && task.configuration_revision)
    return {
      label: "规则控制台",
      href: "./rules.html",
    };
  const revision =
    task.result_ref?.match(/^config:([1-9]\d*)$/)?.[1] ??
    (task.kind === "activation" ? task.configuration_revision : null);
  return revision
    ? {
        label: "配置版本详情",
        href: `./config.html?revision=${revision}&epoch=${encodeURIComponent(epoch)}`,
      }
    : null;
}
const epochPattern =
  /^[a-f\d]{8}-[a-f\d]{4}-[a-f\d]{4}-[a-f\d]{4}-[a-f\d]{12}$/;
export const validEpoch = (value: unknown): value is string =>
  typeof value === "string" && epochPattern.test(value);
export function cursorParts(
  cursor: unknown,
): { epoch: string; sequence: number } | null {
  if (typeof cursor !== "string") return null;
  const [epoch, sequence, ...rest] = cursor.split(":");
  const n = Number(sequence);
  return !rest.length &&
    validEpoch(epoch) &&
    /^\d+$/.test(sequence ?? "") &&
    Number.isSafeInteger(n) &&
    n >= 0 &&
    String(n) === sequence
    ? { epoch, sequence: n }
    : null;
}
const integer = (value: unknown, min = 0): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= min;
const optionalInteger = (value: unknown, min = 0) =>
  value === null || integer(value, min);
const optionalText = (value: unknown, max: number) =>
  value === null || (typeof value === "string" && value.length <= max);
export function validateEventPage(value: unknown): EventsOverview {
  const fail = () => {
    throw new Error("事件服务返回了无效或超限的数据");
  };
  if (!value || typeof value !== "object") return fail();
  const p = value as EventsOverview;
  const cursor = cursorParts(p.cursor);
  if (
    !cursor ||
    cursor.epoch !== p.epoch ||
    !integer(p.head_sequence) ||
    !integer(p.oldest_available_sequence, 1) ||
    cursor.sequence > p.head_sequence ||
    !integer(p.observed_at_unix_ms) ||
    !integer(p.retention_limit, 1) ||
    typeof p.has_more !== "boolean" ||
    (p.has_more && cursor.sequence >= p.head_sequence) ||
    (p.reset_reason !== null &&
      !Object.prototype.hasOwnProperty.call(resetReasons, p.reset_reason)) ||
    !Array.isArray(p.records) ||
    p.records.length > 200
  )
    return fail();
  let previous = 0;
  for (const r of p.records) {
    if (
      !r ||
      !integer(r.sequence, 1) ||
      r.sequence <= previous ||
      r.sequence < p.oldest_available_sequence ||
      r.sequence > cursor.sequence ||
      !validEpoch(r.recorded_epoch) ||
      !integer(r.occurred_at_unix_ms) ||
      !Object.prototype.hasOwnProperty.call(eventLevels, r.level) ||
      typeof r.module !== "string" ||
      r.module.length > 32 ||
      typeof r.code !== "string" ||
      r.code.length > 96 ||
      !optionalInteger(r.task_id, 1) ||
      !optionalInteger(r.task_version, 1) ||
      !optionalInteger(r.configuration_revision, 1) ||
      !optionalText(r.request_id, 64) ||
      !optionalText(r.resource_kind, 32) ||
      !optionalText(r.resource_id, 128) ||
      !optionalText(r.node_id, 128) ||
      !optionalText(r.provider_id, 64) ||
      !optionalText(r.peer, 128) ||
      !optionalText(r.action, 64) ||
      !optionalText(r.outcome, 32)
    )
      return fail();
    previous = r.sequence;
  }
  return p;
}
