import { Badge, Details } from "./ui";
import type { ProviderControllerStatus } from "../types";
export type ControllerAction = "evaluate" | "pause" | "resume";
const reasons: Record<string, string> = {
  disabled: "自动补池未启用",
  idle: "空闲，等待实际代理需求",
  inflight: "已有在途提取，合并触发",
  target_satisfied: "已达到目标库存",
  above_minimum: "高于最低水位",
  budget_exhausted: "每日预算已用尽",
  cooldown: "等待冷却或 Retry-After",
  watermark_capacity: "达到最大可准入库存",
  refill_needed: "需要补充库存",
  paused: "来源提取已暂停",
  health_pending: "库存正在检测，已预留名额",
  health_target_unavailable: "检测目标解析不可用，等待恢复",
  health_budget_exhausted: "健康检测预算不足，暂停自动提取",
  health_source_configuration: "代理认证或检测配置异常，暂停自动提取",
  clock_rollback: "系统时钟回拨，暂停提取",
};
export default function RefillPanel({
  value,
  busy,
  onAction,
  onTemplate,
}: {
  value: ProviderControllerStatus;
  busy: boolean;
  onAction: (action: ControllerAction) => void;
  onTemplate: () => void;
}) {
  return (
    <section className="panel overflow-hidden" aria-label="补池控制器">
      <div className="panel-header panel-toolbar">
        <h2 className="font-semibold">水位与预算控制器</h2>
        <Badge active={value.enabled}>
          {value.enabled
            ? value.mode === "on_demand"
              ? "按需补池"
              : "持续维护"
            : "自动补池停用"}
        </Badge>
      </div>
      <div className="grid grid-cols-2 gap-y-4 divide-x sm:grid-cols-4 divide-console-border py-5 text-center">
        {[
          ["当前准入库存", value.current_inventory],
          ["预测窗口后库存", value.projected_inventory],
          ["在途预留数量", value.inflight_nodes],
          ["等待健康检测", value.pending_health_nodes ?? 0],
        ].map(([label, count]) => (
          <div key={label} className="px-2">
            <p className="muted text-xs">{label}</p>
            <p className="mt-3 text-2xl font-semibold">{count}</p>
          </div>
        ))}
      </div>
      <p className="px-5 pb-4 text-sm" data-testid="controller-decision">
        {reasons[value.decision] ?? value.decision}
        {value.planned_count > 0 ? ` · 计划提取 ${value.planned_count} 个` : ""}
      </p>
      <Details
        rows={[
          [
            "最小 / 目标 / 最大",
            `${value.min_nodes} / ${value.target_nodes} / ${value.max_nodes}`,
          ],
          ["代理需求", value.demand_active ? "当前连接或最近拨号需求" : "空闲"],
          [
            "UTC 每日请求预算",
            `${value.used_requests} / ${value.request_budget}`,
          ],
          [
            "UTC 每日节点预留",
            `${value.reserved_nodes} / ${value.node_budget}`,
          ],
          [
            "预算重置",
            `${new Date(value.resets_at * 1000).toISOString().slice(0, 19).replace("T", " ")} UTC`,
          ],
          [
            "下次允许提取",
            value.next_allowed_at > Date.now() / 1000
              ? new Date(value.next_allowed_at * 1000).toLocaleString("zh-CN")
              : "可评估，仍检查预算与暂停状态",
          ],
          ["连续失败", String(value.failure_count)],
          ["暂停原因", value.paused_reason ?? "无"],
        ]}
      />
      <div className="space-y-3 border-t border-console-border p-5">
        <div className="flex flex-wrap gap-2">
          <button
            className="btn"
            disabled={busy || !value.enabled}
            onClick={() => onAction("evaluate")}
          >
            评估并补充一次
          </button>
          <button
            className="btn"
            disabled={busy || !!value.paused_reason}
            onClick={() => onAction("pause")}
          >
            暂停全部提取
          </button>
          <button
            className="btn"
            disabled={busy || !value.paused_reason}
            onClick={() => onAction("resume")}
          >
            确认结果并恢复提取
          </button>
          <button className="btn" disabled={busy} onClick={onTemplate}>
            加入补池配置模板
          </button>
        </div>
        <p className="muted text-xs leading-6">
          恢复保留已预留的预算、最小间隔和
          Retry-After。结果不确定时请先核对供应商记录，恢复后可能产生新的提取费用。暂停不取消已发送的请求，也不关闭已有代理连接。
        </p>
        <p className="muted text-xs leading-6">
          当前库存依据来源、期限与配置准入，健康状态由健康模块接续。按需模式在无连接或最近拨号需求时停止自动购买；持续维护需要显式启用。参数编辑在下方
          JSON 的 refill 字段，保存后才生效。
        </p>
      </div>
    </section>
  );
}
