import { useCallback, useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import type { ServiceData } from "../use-service";
import type { LinuxPreflight } from "../types";
import { Badge } from "../components/ui";
import { getJson } from "../platform";
import LinuxDiagnosticsPanel from "../components/LinuxDiagnosticsPanel";
import CapturePanel from "./CapturePanel";
const statuses = {
  pass: "条件通过",
  fail: "条件不足",
  unknown: "需要确认",
  unsupported: "不支持",
};
export default function LinuxPage({
  data,
  refreshService,
}: {
  data: ServiceData;
  refreshService: () => Promise<void>;
}) {
  const [preflight, setPreflight] = useState<LinuxPreflight | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const refresh = useCallback(() => setAttempt((value) => value + 1), []);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    void getJson<LinuxPreflight>("api/v1/linux/preflight", controller.signal)
      .then((value) => {
        if (!controller.signal.aborted) {
          setPreflight(value);
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
  return (
    <div className="space-y-3">
      <CapturePanel
        key={`${data.endpoint}:${data.status.instance_id}`}
        refreshService={refreshService}
      />
      <LinuxDiagnosticsPanel
        key={`${data.endpoint}:${data.status.instance_id}`}
      />
      <section className="panel overflow-hidden" aria-label="Linux 环境预检">
        <div className="panel-header panel-toolbar">
          <div>
            <h2 className="font-semibold">目标服务环境预检</h2>
            <p className="muted mt-2 text-xs">
              只读检查，不加载程序或修改系统网络。
            </p>
          </div>
          <button className="btn" disabled={loading} onClick={refresh}>
            <span
              className="action-icon inline-flex"
              data-spinning={loading}
              aria-hidden="true"
            >
              <RefreshCw size={15} />
            </span>
            刷新预检
          </button>
        </div>
        {error && (
          <p
            role="alert"
            className="p-5 text-sm text-red-600 dark:text-red-300"
          >
            {error}
          </p>
        )}
        {preflight ? (
          <>
            <p className="muted border-b border-console-border px-5 py-4 text-xs">
              平台：{preflight.platform} · 内核：
              {preflight.kernel_release ?? "不适用"} · {preflight.scope}
            </p>
            <div className="divide-y divide-console-border">
              {preflight.checks.map((check) => (
                <div key={check.id} className="p-5">
                  <div className="flex items-center justify-between gap-3">
                    <strong className="font-mono text-xs">{check.id}</strong>
                    <Badge active={check.status === "pass"}>
                      {statuses[check.status]}
                    </Badge>
                  </div>
                  <p className="muted mt-3 break-words text-sm leading-6">
                    {check.detail}
                  </p>
                </div>
              ))}
            </div>
          </>
        ) : (
          <p className="muted p-6 text-sm">正在读取目标服务内核条件…</p>
        )}
      </section>
      <section className="panel panel-body">
        <h2 className="font-semibold">本机隔离验证</h2>
        <p className="muted mt-3 text-sm leading-7">
          在目标 Linux 服务器本地运行
          CLI，使用本项目编译的对象和系统所需能力验证真实 hook、map
          更新与回收。它只操作新建的测试
          cgroup，不启用生产流量接管；远端服务地址不会把该命令变成远程执行。
        </p>
        <pre className="code-block mt-4">
          <code>
            hps --output json linux self-test --object /opt/hps/hps-ebpf
          </code>
        </pre>
        <p className="muted mt-3 text-xs leading-6">
          默认 cgroup 根目录为 /sys/fs/cgroup，可用 --cgroup-root 指定真实
          cgroup v2
          挂载。持久接管与故障恢复由运行配置单独管理，不能以预检或隔离自测代替。
        </p>
      </section>
    </div>
  );
}
