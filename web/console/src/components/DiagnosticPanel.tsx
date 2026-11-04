import { useEffect, useRef, useState } from "react";
import { getJson, postJson, saveDiagnostic } from "../platform";
import type { TaskInfo } from "../tasks";
import type { ServiceStatus } from "../types";

type Controls = {
  version: number;
  can_cancel: boolean;
  can_retry: boolean;
  reason: string;
  retry_of: number | null;
  result_persistence_failed: boolean;
};
export function TaskControlPanel({
  task,
  epoch,
  onChanged,
}: {
  task: TaskInfo;
  epoch: string;
  onChanged: (task: TaskInfo) => void;
}) {
  const [controls, setControls] = useState<Controls>(),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const [retryId, setRetryId] = useState(() => crypto.randomUUID());
  useEffect(() => {
    const abort = new AbortController();
    setControls(undefined);
    setError("");
    void getJson<Controls>(
      `api/v1/tasks/${task.id}/controls?expected_epoch=${encodeURIComponent(epoch)}`,
      abort.signal,
    )
      .then((v) => {
        if (!abort.signal.aborted) setControls(v);
      })
      .catch((e) => {
        if (!abort.signal.aborted) setError(String(e));
      });
    return () => abort.abort();
  }, [task.id, task.version, epoch]);
  async function action(kind: "cancel" | "retry") {
    if (!controls || busy) return;
    setBusy(true);
    setError("");
    try {
      const value = await postJson<TaskInfo>(
        `api/v1/tasks/${task.id}/${kind}`,
        {
          expected_epoch: epoch,
          expected_version: controls.version,
          ...(kind === "retry" ? { request_id: retryId } : {}),
        },
      );
      setRetryId(crypto.randomUUID());
      onChanged(value);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div
      className="space-y-2 rounded-xl border border-slate-200 p-4 dark:border-slate-700"
      role="group"
      aria-label="任务控制"
    >
      <p className="text-xs">
        取消与重试：{controls?.reason ?? "正在读取当前阶段能力…"}
      </p>
      {controls?.retry_of && (
        <p className="muted text-xs">源任务 #{controls.retry_of}</p>
      )}
      {controls?.result_persistence_failed && (
        <p role="alert">结果持久化失败；不能视作成功。</p>
      )}
      <div className="flex flex-wrap gap-2">
        {controls?.can_cancel && (
          <button
            disabled={busy}
            className="btn"
            onClick={() => void action("cancel")}
          >
            取消只读诊断
          </button>
        )}
        {controls?.can_retry && (
          <button
            disabled={busy}
            className="btn"
            onClick={() => void action("retry")}
          >
            创建诊断重试任务
          </button>
        )}
        {task.kind === "diagnostic_export" && task.state === "completed" && (
          <a
            className="btn"
            href={`./events.html?diagnostic=${task.id}&epoch=${encodeURIComponent(epoch)}`}
          >
            预览并导出诊断包
          </a>
        )}
      </div>
      {error && (
        <p role="alert" className="break-all text-xs text-red-600">
          {error}
        </p>
      )}
    </div>
  );
}
export default function DiagnosticPanel() {
  const query = new URLSearchParams(window.location.search);
  const id = query.get("diagnostic"),
    epoch = query.get("epoch");
  const [windowSecs, setWindow] = useState(300),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [info, setInfo] = useState("");
  const [bundle, setBundle] = useState<unknown>(),
    [task, setTask] = useState<TaskInfo>();
  const mounted = useRef(true);
  const request = useRef<{ id: string; epoch: string; window: number } | null>(
    query.get("diagnostic_request") && epoch
      ? {
          id: query.get("diagnostic_request")!,
          epoch,
          window: Number(query.get("diagnostic_window") ?? 300),
        }
      : null,
  );
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (!id || !epoch) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function read() {
      try {
        const value = await getJson<TaskInfo>(
          `api/v1/tasks/${encodeURIComponent(id!)}?expected_epoch=${encodeURIComponent(epoch!)}`,
          controller.signal,
        );
        if (controller.signal.aborted) return;
        setTask(value);
        if (value.state === "completed") {
          const data = await getJson<unknown>(
            `api/v1/diagnostics/${encodeURIComponent(id!)}?expected_epoch=${encodeURIComponent(epoch!)}`,
            controller.signal,
          );
          if (!controller.signal.aborted) setBundle(data);
        } else if (!value.terminal) timer = setTimeout(() => void read(), 500);
      } catch (e) {
        if (!controller.signal.aborted) setError(String(e));
      }
    }
    void read();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [id, epoch]);
  async function start() {
    setBusy(true);
    setError("");
    try {
      if (!request.current) {
        const status = await getJson<ServiceStatus>("api/v1/status");
        request.current = {
          id: crypto.randomUUID(),
          epoch: status.instance_id,
          window: windowSecs,
        };
      }
      const ticket = request.current;
      const recovery = new URLSearchParams(window.location.search);
      recovery.set("diagnostic_request", ticket.id);
      recovery.set("epoch", ticket.epoch);
      recovery.set("diagnostic_window", String(ticket.window));
      window.history.replaceState(null, "", `?${recovery}`);
      const task = await postJson<TaskInfo>("api/v1/diagnostics", {
        expected_epoch: ticket.epoch,
        request_id: ticket.id,
        window_secs: ticket.window,
      });
      if (mounted.current)
        window.location.assign(
          `./events.html?diagnostic=${task.id}&epoch=${encodeURIComponent(ticket.epoch)}`,
        );
    } catch (e) {
      if (mounted.current) setError(String(e));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <section className="panel space-y-4 p-5" aria-label="脱敏诊断包">
      <h2 className="font-semibold">脱敏诊断包</h2>
      <p className="muted text-xs leading-6">
        后台收集版本、能力、计数和状态元数据。窗口 1–60
        分钟，历史有界；不导出代理地址、供应商凭据、配置正文或业务内容。此文件不是数据库一致性备份。离开页面不会终止已接受任务。
      </p>
      <div className="flex flex-wrap items-center gap-3">
        <label className="form-field">
          诊断窗口{" "}
          <select
            aria-label="诊断窗口"
            className="field"
            disabled={busy || !!request.current}
            value={windowSecs}
            onChange={(e) => setWindow(Number(e.target.value))}
          >
            <option value={60}>1 分钟</option>
            <option value={300}>5 分钟</option>
            <option value={3600}>1 小时</option>
          </select>
        </label>
        <button className="btn" disabled={busy} onClick={() => void start()}>
          {busy
            ? "正在提交…"
            : request.current
              ? "按原请求恢复提交"
              : "创建诊断包"}
        </button>
        {request.current && !busy && (
          <>
            <a
              className="btn"
              href={`./tasks.html?request_id=${encodeURIComponent(request.current.id)}&epoch=${encodeURIComponent(request.current.epoch)}`}
            >
              按原请求查询任务
            </a>
            <button
              className="btn"
              onClick={() => window.location.assign("./events.html")}
            >
              重新开始诊断请求
            </button>
          </>
        )}
        {task && (
          <a
            className="btn"
            href={`./tasks.html?detail=${task.id}&epoch=${encodeURIComponent(epoch!)}`}
          >
            诊断任务 #{task.id} · {task.state}
          </a>
        )}
        {bundle !== undefined && (
          <button
            className="btn"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void saveDiagnostic(Number(id), epoch!)
                .then((saved) => {
                  if (mounted.current)
                    setInfo(saved ? "诊断文件已导出" : "已取消文件保存");
                })
                .catch((e) => {
                  if (mounted.current) setError(String(e));
                })
                .finally(() => {
                  if (mounted.current) setBusy(false);
                });
            }}
          >
            导出诊断 JSON
          </button>
        )}
      </div>
      {bundle !== undefined && (
        <details open>
          <summary className="text-sm">导出内容预览</summary>
          <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-all p-3 text-xs">
            {JSON.stringify(bundle, null, 2)}
          </pre>
        </details>
      )}
      {error && (
        <p role="alert" className="break-all text-xs text-red-600">
          {error}
        </p>
      )}
      {info && (
        <p role="status" className="text-xs">
          {info}
        </p>
      )}
    </section>
  );
}
