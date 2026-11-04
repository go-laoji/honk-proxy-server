import { useEffect, useState } from "react";
import { getJson } from "../platform";
type Snapshot = {
  epoch: string;
  scope: string;
  topology: {
    supported: boolean;
    kernel_release: string | null;
    network_namespace: string | null;
    cgroup_namespace: string | null;
    cgroup_membership: string | null;
    interfaces: {
      name: string;
      ifindex: number | null;
      mtu: number | null;
      state: string | null;
      master: string | null;
    }[];
    routes: {
      family: string;
      interface: string;
      destination: string;
      gateway: string;
      metric: number;
    }[];
    mounts: { path: string; filesystem: string; options: string }[];
    warnings: string[];
    truncated: boolean;
  };
  capture: {
    state: string;
    ready: boolean;
    program_ids: number[];
    map_ids: number[];
    kernel_revision: number | null;
    userspace_revision: number;
    flow_entries: number;
    active_owners: number;
    checked_at_unix_ms: number;
    hooks: {
      hook: string;
      captured_packets: number;
      bypassed_packets: number;
      rejected_packets: number;
      errors: number;
    }[];
  };
};
export default function LinuxDiagnosticsPanel() {
  const [{ current, previous }, setSnapshot] = useState<{
    current: Snapshot | null;
    previous: Snapshot | null;
  }>({ current: null, previous: null });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    void getJson<Snapshot>("api/v1/linux/diagnostics", controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) {
          setSnapshot((old) => ({ previous: old.current, current: value }));
          setError("");
        }
      })
      .catch((cause) => {
        if (!controller.signal.aborted)
          setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [attempt]);
  const comparable =
    current &&
    previous &&
    current.epoch === previous.epoch &&
    current.capture.kernel_revision === previous.capture.kernel_revision &&
    JSON.stringify(current.capture.program_ids) ===
      JSON.stringify(previous.capture.program_ids) &&
    JSON.stringify(current.capture.map_ids) ===
      JSON.stringify(previous.capture.map_ids);
  return (
    <section className="panel panel-body" aria-label="只读拓扑与内核诊断">
      <div className="flex items-center justify-between gap-3">
        <h2 className="font-semibold">只读拓扑与内核诊断</h2>
        <button
          className="btn"
          disabled={loading}
          onClick={() => setAttempt((v) => v + 1)}
        >
          {loading ? "正在观测…" : "刷新并比较"}
        </button>
      </div>
      <p className="muted mt-3 text-xs leading-6">
        读取目标 Daemon
        当前命名空间与本实例已加载对象；不加载程序、不切换命名空间、不更改路由。计数变化仅比较同一
        epoch 和对象代次。
      </p>
      {error && (
        <p role="alert" className="mt-3 text-red-600">
          {error}
        </p>
      )}
      {current && (
        <div className="mt-4 space-y-4 text-sm">
          <p>
            内核 {current.topology.kernel_release ?? "不支持"} · 网络{" "}
            {current.topology.network_namespace ?? "不可读"} · cgroup{" "}
            {current.topology.cgroup_namespace ?? "不可读"}
          </p>
          <pre className="overflow-auto rounded-lg bg-console-soft p-3 text-xs">
            {current.topology.cgroup_membership ??
              "当前平台不提供 Linux cgroup"}
          </pre>
          <div className="overflow-auto">
            <table className="w-full text-left text-xs">
              <thead>
                <tr>
                  <th>接口</th>
                  <th>索引</th>
                  <th>MTU</th>
                  <th>状态</th>
                  <th>主接口</th>
                </tr>
              </thead>
              <tbody>
                {current.topology.interfaces.map((item) => (
                  <tr key={item.name}>
                    <td className="py-2">{item.name}</td>
                    <td>{item.ifindex ?? "?"}</td>
                    <td>{item.mtu ?? "?"}</td>
                    <td>{item.state ?? "?"}</td>
                    <td>{item.master ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="overflow-auto">
            <table className="w-full text-left text-xs">
              <thead>
                <tr>
                  <th>目的地址</th>
                  <th>网关</th>
                  <th>接口</th>
                  <th>Metric</th>
                </tr>
              </thead>
              <tbody>
                {current.topology.routes.map((item, index) => (
                  <tr key={index}>
                    <td className="py-2 font-mono">{item.destination}</td>
                    <td>{item.gateway}</td>
                    <td>{item.interface}</td>
                    <td>{item.metric}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="break-all">
            已持有程序 IDs：{current.capture.program_ids.join(", ") || "无"} ·
            maps：{current.capture.map_ids.join(", ") || "无"}
          </p>
          <p>
            内核 / 用户态 revision：
            {current.capture.kernel_revision ?? "未发布"} /{" "}
            {current.capture.userspace_revision} · flow entries{" "}
            {current.capture.flow_entries} · owners{" "}
            {current.capture.active_owners}
          </p>
          {previous && !comparable && (
            <p role="status">epoch 或对象代次已变化，已重置计数比较。</p>
          )}
          <div className="overflow-auto">
            <table className="w-full text-left text-xs">
              <thead>
                <tr>
                  <th>Hook</th>
                  <th>捕获</th>
                  <th>绕过</th>
                  <th>拒绝</th>
                  <th>错误</th>
                </tr>
              </thead>
              <tbody>
                {current.capture.hooks.map((item) => {
                  const old = comparable
                    ? previous?.capture.hooks.find((v) => v.hook === item.hook)
                    : undefined;
                  const value = (
                    key:
                      | "captured_packets"
                      | "bypassed_packets"
                      | "rejected_packets"
                      | "errors",
                  ) =>
                    `${item[key]}${old ? (item[key] >= old[key] ? ` (+${item[key] - old[key]})` : " (计数重置)") : ""}`;
                  return (
                    <tr key={item.hook}>
                      <td className="py-2">{item.hook}</td>
                      <td>{value("captured_packets")}</td>
                      <td>{value("bypassed_packets")}</td>
                      <td>{value("rejected_packets")}</td>
                      <td>{value("errors")}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          {current.topology.mounts.map((item) => (
            <p key={item.path} className="break-all text-xs">
              {item.filesystem} · {item.path} · {item.options}
            </p>
          ))}
          {current.topology.truncated && (
            <p role="status">结果达到有界容量，部分条目未显示。</p>
          )}
          {current.topology.warnings.map((item) => (
            <p key={item} className="muted text-xs leading-6">
              {item}
            </p>
          ))}
        </div>
      )}
    </section>
  );
}
