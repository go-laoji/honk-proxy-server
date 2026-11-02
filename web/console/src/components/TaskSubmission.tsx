import { useEffect, useRef, useState, type FormEvent } from "react";
import { FileCode2, Send, X } from "lucide-react";
import { ApiRequestError, getJson, postJson } from "../platform";
import type { TaskInfo } from "../tasks";
import type { ConfigWorkspace, ServiceStatus } from "../types";

export default function TaskSubmission({
  onRequest,
  onAccepted,
}: {
  onRequest: (id: string, epoch: string) => void;
  onAccepted: (task: TaskInfo, epoch: string) => void;
}) {
  const [open, setOpen] = useState(false),
    [loading, setLoading] = useState(false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const [body, setBody] = useState(""),
    [revision, setRevision] = useState(0),
    [epoch, setEpoch] = useState("");
  const [requestId, setRequestId] = useState<string>(crypto.randomUUID()),
    [prepare, setPrepare] = useState(false),
    [accepted, setAccepted] = useState<TaskInfo | null>(null);
  const mounted = useRef(true);
  const read = useRef<AbortController | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      read.current?.abort();
    };
  }, []);
  async function show() {
    setError("");
    setLoading(true);
    read.current?.abort();
    const controller = new AbortController();
    read.current = controller;
    try {
      const status = await getJson<ServiceStatus>(
        "api/v1/status",
        controller.signal,
      );
      const workspace = await getJson<ConfigWorkspace>(
        "api/v1/config/workspace",
        controller.signal,
      );
      if (controller.signal.aborted || !mounted.current) return;
      setBody(workspace.desired_toml);
      setRevision(workspace.desired_revision);
      setEpoch(status.instance_id);
      setRequestId(crypto.randomUUID());
      setAccepted(null);
      setOpen(true);
    } catch (cause) {
      if (mounted.current && !controller.signal.aborted)
        setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (mounted.current && !controller.signal.aborted) setLoading(false);
    }
  }
  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError("");
    // The URL retains only the recovery key and epoch; configuration text is never put in local storage.
    onRequest(requestId, epoch);
    try {
      const task = await postJson<TaskInfo>("api/v1/tasks", {
        expected_epoch: epoch,
        request_id: requestId,
        operation: {
          kind: prepare ? "configuration_prepare" : "configuration_validate",
          input: {
            toml: body,
            expected_revision: revision,
            expected_draft_version: null,
            note: prepare
              ? "Console asynchronous revision"
              : "Console asynchronous validation",
            use_draft_credentials: false,
          },
        },
      });
      if (mounted.current) {
        setAccepted(task);
        onAccepted(task, epoch);
      }
    } catch (cause) {
      if (mounted.current)
        setError(
          cause instanceof ApiRequestError &&
            cause.code === "configuration_conflict"
            ? "提交冲突：配置版本已变化，或同一请求 ID 对应的动作/输入不同。先查询原请求；改动输入时使用新的请求 ID。"
            : cause instanceof ApiRequestError &&
                cause.code === "task_instance_changed"
              ? "服务实例已变化，请重新读取配置后提交。"
              : cause instanceof Error
                ? cause.message
                : String(cause),
        );
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <section className="panel space-y-4 p-5" aria-label="后台配置提交">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="flex items-center gap-2 font-semibold">
            <FileCode2 size={18} />
            可恢复的配置任务
          </h2>
          <p className="muted mt-2 text-xs leading-6">
            离开页面后由服务继续执行。生成候选不会激活配置；配置工作台仍可提供逐字段校验与激活。
          </p>
        </div>
        <button
          className="btn"
          disabled={loading || busy}
          onClick={() => (open ? setOpen(false) : void show())}
        >
          {open ? <X size={14} /> : <FileCode2 size={14} />}
          {loading
            ? "正在读取配置…"
            : open
              ? "收起提交表单"
              : "后台提交配置任务"}
        </button>
      </div>
      {open && (
        <form className="space-y-4" onSubmit={(event) => void submit(event)}>
          <p className="muted break-all text-xs">
            期望版本 r{revision} · 实例 {epoch}
            。服务实例或配置变化会返回冲突；相同请求 ID
            不重复执行，改变输入须使用新的请求 ID。
          </p>
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="text-xs">
              任务动作
              <select
                className="field mt-1"
                disabled={busy}
                value={prepare ? "prepare" : "validate"}
                onChange={(e) => setPrepare(e.target.value === "prepare")}
              >
                <option value="validate">仅配置校验</option>
                <option value="prepare">预检并保存未激活候选</option>
              </select>
            </label>
            <label className="text-xs">
              幂等请求 ID
              <input
                className="field mt-1"
                required
                minLength={1}
                maxLength={64}
                pattern="[A-Za-z0-9_.:\-]+"
                disabled={busy}
                value={requestId}
                onChange={(e) => setRequestId(e.target.value)}
              />
            </label>
          </div>
          <label className="block text-xs">
            后台配置 TOML
            <textarea
              className="field mt-1 min-h-64 font-mono text-xs"
              disabled={busy}
              value={body}
              onChange={(e) => setBody(e.target.value)}
            />
          </label>
          <div className="flex flex-wrap gap-2">
            <button className="btn" disabled={busy || !revision}>
              <Send size={14} />
              {busy ? "正在提交…" : "提交到后台"}
            </button>
            <button
              type="button"
              className="btn"
              disabled={busy}
              onClick={() => {
                setRequestId(crypto.randomUUID());
                setAccepted(null);
              }}
            >
              生成新的请求 ID
            </button>
            <a className="py-2 text-xs text-blue-600" href="./config.html">
              配置工作台 →
            </a>
          </div>
          {accepted && (
            <p role="status" className="break-words text-sm text-emerald-700">
              任务 #{accepted.id}{" "}
              已接受。终态与结果以任务详情为准；同一请求再次提交会返回该任务。
            </p>
          )}
        </form>
      )}
      {error && (
        <p role="alert" className="break-words text-sm text-red-600">
          {error}。若等待中断，可按页面中的同一请求 ID 查询；不要盲目换 ID
          重交。
        </p>
      )}
    </section>
  );
}
