import { Badge } from "./ui";
import {
  nodeAddress,
  nodeLocationLabel,
  nodeLocationSource,
  nodeReasons,
  nodeStates,
  type NodeView,
} from "../nodes";

const actions = [
  ["enable", "解除限制", "enabled"],
  ["disable", "禁用", "disabled"],
  ["drain", "排空", "draining"],
] as const;
export type NodeAction = (typeof actions)[number][0] | "delete" | "remove";

type ActionProps = {
  disabled: boolean;
  onAction: (node: NodeView, action: NodeAction) => void;
};

export function NodeActions({
  node,
  disabled,
  onAction,
}: ActionProps & { node: NodeView }) {
  const canOperate = node.kind !== "lease" || node.lease_state === "active";
  return (
    <div className="flex flex-wrap gap-1">
      {canOperate &&
        actions.map(([action, label, state]) => (
          <button
            type="button"
            key={action}
            className="btn min-h-7 px-2 py-0.5 text-xs whitespace-nowrap"
            disabled={disabled || node.operation.state === state}
            onClick={() => onAction(node, action)}
          >
            {label}
          </button>
        ))}
      <button
        type="button"
        className="btn min-h-7 px-2 py-0.5 text-xs whitespace-nowrap text-console-danger"
        disabled={
          disabled ||
          (node.kind !== "static" && node.operation.state === "deleted")
        }
        onClick={() =>
          onAction(node, node.kind === "static" ? "remove" : "delete")
        }
      >
        删除
      </button>
    </div>
  );
}

function lifetime(node: NodeView): string {
  if (node.remaining_secs === null) {
    return node.kind === "subscription" ? "有效期未知" : "未设到期";
  }
  return node.remaining_secs === 0 ? "已到期" : `${node.remaining_secs} 秒`;
}

export default function NodeTable({
  nodes,
  disabled,
  onAction,
}: ActionProps & { nodes: NodeView[] }) {
  return (
    <div
      className="node-table-scroll overflow-x-auto"
      role="region"
      aria-label="节点表格滚动区"
      tabIndex={0}
    >
      <table className="node-table" aria-label="节点列表">
        <colgroup>
          <col className="w-[19%]" />
          <col className="w-[10%]" />
          <col className="w-[14%]" />
          <col className="w-[10%]" />
          <col className="w-[8%]" />
          <col className="w-[9%]" />
          <col className="w-[5%]" />
          <col className="w-[25%]" />
        </colgroup>
        <thead>
          <tr>
            {[
              "节点 / 端点",
              "来源",
              "归属地",
              "有效期",
              "准入",
              "健康",
              "连接",
              "操作",
            ].map((label) => (
              <th key={label} scope="col">
                {label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {nodes.map((node) => {
            const address = nodeAddress(node);
            const source = node.provider_id ?? "静态节点";
            const lease = nodeStates[node.lease_state] ?? node.lease_state;
            const location = node.location;
            const region = location
              ? nodeLocationLabel(location)
              : (node.region ?? "未知");
            const regionSource = location
              ? [nodeLocationSource(location), location.isp]
                  .filter(Boolean)
                  .join(" · ")
              : node.region
                ? "供应商"
                : null;
            const reasons = node.admission_reasons.map(
              (reason) => nodeReasons[reason] ?? reason,
            );
            return (
              <tr key={node.id} aria-label={`节点 ${node.id}`}>
                <th scope="row">
                  <a
                    className="node-table-link block truncate"
                    href={`./node-detail.html?id=${encodeURIComponent(node.id)}`}
                    title={node.id}
                  >
                    {node.id}
                  </a>
                  <span
                    className="muted block truncate font-mono font-normal"
                    title={address}
                  >
                    {address}
                  </span>
                </th>
                <td>
                  <span className="block truncate" title={source}>
                    {source}
                  </span>
                  {node.batch_id !== null && (
                    <span className="muted">批次 #{node.batch_id}</span>
                  )}
                </td>
                <td>
                  <span className="block truncate" title={region}>
                    {region}
                  </span>
                  {regionSource && (
                    <span
                      className="muted block truncate"
                      title={
                        location
                          ? `${regionSource} · ${location.ip}`
                          : regionSource
                      }
                    >
                      {regionSource}
                    </span>
                  )}
                </td>
                <td className="tabular-nums">
                  <span className="whitespace-nowrap">{lifetime(node)}</span>
                  {node.kind === "lease" && node.remaining_secs !== 0 && (
                    <span className="muted block truncate" title={lease}>
                      {lease}
                    </span>
                  )}
                </td>
                <td>
                  <Badge active={node.eligible}>
                    {node.eligible ? "可准入" : "不可准入"}
                  </Badge>
                  {reasons.length > 0 && (
                    <span
                      className="muted block truncate"
                      title={reasons.join(" · ")}
                    >
                      {reasons[0]}
                    </span>
                  )}
                </td>
                <td>
                  <span
                    className="block truncate"
                    title={nodeStates[node.health_state] ?? node.health_state}
                  >
                    {nodeStates[node.health_state] ?? node.health_state}
                  </span>
                </td>
                <td className="tabular-nums">{node.active_connections}</td>
                <td>
                  <NodeActions
                    node={node}
                    disabled={disabled}
                    onAction={onAction}
                  />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
