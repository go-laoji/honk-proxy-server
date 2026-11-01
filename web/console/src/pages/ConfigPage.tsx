import { useEffect, useRef, useState } from "react";
import {
  FileCode2,
  GitBranch,
  Play,
  RotateCcw,
  Download,
  Upload,
  CheckCheck,
  Save,
  RefreshCw,
  Database,
} from "lucide-react";
import { Badge, Details, Stat } from "../components/ui";
import { getJson, postJson, saveConfiguration } from "../platform";
import type {
  ConfigRevision,
  ConfigValidation,
  ConfigWorkspace,
} from "../types";

const date = (time: number) => new Date(time * 1000).toLocaleString("zh-CN");
const preview = (value: unknown) =>
  JSON.stringify(value, null, 2)?.slice(0, 5000) ?? "null";
const message = (error: unknown) =>
  error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : JSON.stringify(error);

export default function ConfigPage({
  refreshService,
}: {
  refreshService: () => Promise<void>;
}) {
  const [workspace, setWorkspace] = useState<ConfigWorkspace | null>(null);
  const [editor, setEditor] = useState("");
  const [useDraftCredentials, setUseDraftCredentials] = useState(true);
  const [note, setNote] = useState("");
  const [validation, setValidation] = useState<ConfigValidation | null>(null);
  const [validated, setValidated] = useState<{
    text: string;
    head: number;
  } | null>(null);
  const [saved, setSaved] = useState<ConfigRevision | null>(null);
  const [pending, setPending] = useState("");
  const [error, setError] = useState("");
  const [info, setInfo] = useState("");
  const mounted = useRef(true);
  const fileInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    mounted.current = true;
    const controller = new AbortController();
    void getJson<ConfigWorkspace>("api/v1/config/workspace", controller.signal)
      .then((state) => {
        if (!controller.signal.aborted) {
          setWorkspace(state);
          setEditor(state.draft.toml);
        }
      })
      .catch((cause) => {
        if (!controller.signal.aborted) setError(message(cause));
      });
    return () => {
      mounted.current = false;
      controller.abort();
    };
  }, []);
  async function reload(replaceEditor = false) {
    const state = await getJson<ConfigWorkspace>("api/v1/config/workspace");
    if (mounted.current) {
      setWorkspace(state);
      setValidation(null);
      setValidated(null);
      if (replaceEditor) {
        setEditor(state.draft.toml);
        setUseDraftCredentials(true);
      }
    }
    return state;
  }
  async function run(label: string, operation: () => Promise<void>) {
    setPending(label);
    setError("");
    setInfo("");
    try {
      await operation();
    } catch (cause) {
      if (mounted.current) setError(message(cause));
    } finally {
      if (mounted.current) setPending("");
    }
  }
  function body() {
    return {
      toml: editor,
      expected_revision: workspace!.desired_revision,
      expected_draft_version: workspace!.draft.version,
      note: note || "控制台保存",
      use_draft_credentials: useDraftCredentials,
    };
  }
  async function validate() {
    const result = await postJson<ConfigValidation>(
      "api/v1/config/validate",
      body(),
    );
    if (mounted.current) {
      setValidation(result);
      setValidated({ text: editor, head: workspace!.desired_revision });
      setInfo(
        result.valid
          ? "校验通过。请审阅差异后保存版本，再激活。"
          : "配置校验失败，请修改下方标记的问题。",
      );
    }
  }
  async function activate(revision: number, rollback = false) {
    const result = await postJson<{
      activation_id: number;
      state: string;
      restart_required: string[];
    }>(`api/v1/config/${rollback ? "rollback" : "activate"}`, {
      revision,
      expected_revision: workspace!.desired_revision,
    });
    await reload();
    await refreshService();
    if (mounted.current)
      setInfo(
        `激活记录 #${result.activation_id}：${result.state === "applied" ? "已应用到实际运行代际" : `已保存期望版本，需重启 Daemon：${result.restart_required.join("、")}`}。`,
      );
  }
  async function importFile(file: File) {
    if (file.size > 2 * 1024 * 1024) throw new Error("导入文件不能超过 2 MiB");
    const text = await file.text();
    if (mounted.current) {
      setEditor(text);
      setUseDraftCredentials(false);
      setValidation(null);
      setValidated(null);
      setSaved(null);
      setInfo("文件已载入编辑器。先校验和审阅差异，尚未保存或激活。");
    }
  }
  if (!workspace)
    return (
      <section className="panel p-8">
        <p role={error ? "alert" : "status"}>
          {error || "正在读取共享配置工作台…"}
        </p>
      </section>
    );
  const valid = Boolean(
    validation?.valid &&
    validated?.text === editor &&
    validated.head === workspace.desired_revision,
  );
  return (
    <>
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <Stat
          title="实际运行版本"
          value={`r${workspace.running_revision}`}
          detail="以当前进程实际代际为准"
          icon={Play}
        />
        <Stat
          title="期望版本"
          value={`r${workspace.desired_revision}`}
          detail={
            workspace.pending_restart.length
              ? "部分设置等待重启"
              : "期望与运行状态分别记录"
          }
          icon={GitBranch}
        />
        <Stat
          title="共享草稿"
          value={`v${workspace.draft.version}`}
          detail={`基于 r${workspace.draft.base_revision}，所有使用者共享`}
          icon={FileCode2}
        />
        <Stat
          title="排空代际"
          value={String(workspace.draining_generations)}
          detail="已有连接按原配置继续完成"
          icon={Database}
        />
      </div>
      {workspace.pending_restart.length > 0 && (
        <p className="mt-5 rounded-xl border border-amber-200 bg-amber-50 p-4 text-sm leading-7 text-amber-900 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
          期望配置尚需重启：{workspace.pending_restart.join("、")}。当前仍运行 r
          {workspace.running_revision}，不会将保存成功显示为已运行。
        </p>
      )}
      {error && (
        <div
          role="alert"
          className="mt-5 rounded-xl bg-red-50 p-4 text-sm leading-7 text-red-700 dark:bg-red-950 dark:text-red-200"
        >
          {error}
          <p>
            版本冲突时请重新载入并再次校验。操作结果以最新版本和激活记录为准。
          </p>
        </div>
      )}
      {info && (
        <p
          role="status"
          className="mt-5 text-sm leading-7 text-blue-700 dark:text-blue-300"
        >
          {info}
        </p>
      )}
      <section className="panel mt-6 min-w-0">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-100 p-5 dark:border-slate-800">
          <div>
            <h2 className="font-semibold">共享配置编辑器</h2>
            <p className="muted mt-1 text-xs">
              TOML · 格式 v1 · 默认脱敏 · 写入携带期望版本
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            <button
              className="btn"
              disabled={Boolean(pending)}
              onClick={() =>
                void run("reload", async () => {
                  await reload(true);
                  setSaved(null);
                  setInfo("已重新载入共享草稿和最新版本。");
                })
              }
            >
              <RefreshCw size={14} />
              重新载入草稿
            </button>
            <button
              className="btn"
              disabled={Boolean(pending)}
              onClick={() => {
                setEditor(workspace.desired_toml);
                setUseDraftCredentials(false);
                setValidation(null);
                setValidated(null);
                setSaved(null);
              }}
            >
              载入期望配置
            </button>
            <button
              className="btn"
              disabled={Boolean(pending)}
              onClick={() => fileInput.current?.click()}
            >
              <Upload size={14} />
              导入文件
            </button>
            <input
              ref={fileInput}
              type="file"
              accept=".toml,text/plain"
              className="hidden"
              aria-label="导入 TOML 文件"
              onChange={(event) => {
                const file = event.target.files?.[0];
                if (file) void run("import", () => importFile(file));
                event.target.value = "";
              }}
            />
            <button
              className="btn"
              disabled={Boolean(pending)}
              onClick={() =>
                void run("export", async () => {
                  if (await saveConfiguration(workspace.desired_revision))
                    setInfo("已导出脱敏的期望配置。");
                })
              }
            >
              <Download size={14} />
              导出脱敏配置
            </button>
          </div>
        </div>
        <div className="p-5">
          <label
            htmlFor="config-toml"
            className="mb-2 block text-sm font-medium"
          >
            配置 TOML
          </label>
          <textarea
            id="config-toml"
            spellCheck={false}
            value={editor}
            disabled={Boolean(pending)}
            onChange={(event) => setEditor(event.target.value)}
            className="min-h-[25rem] w-full resize-y rounded-xl border border-slate-200 bg-slate-950 p-4 font-mono text-xs leading-6 text-slate-100 outline-none focus:border-blue-500 dark:border-slate-700"
          />
          <p className="muted mt-3 text-xs leading-6">
            [REDACTED] 仅保留同 ID
            已有上游凭据；新节点需填写实际凭据或不配置认证。导入的相对证书路径基于服务数据目录，文件须位于
            Daemon 主机。载入配置会替换编辑器中的未保存内容。
          </p>
          <div className="mt-4 flex flex-wrap items-center gap-3">
            <label className="sr-only" htmlFor="revision-note">
              版本说明
            </label>
            <input
              id="revision-note"
              value={note}
              onChange={(event) => setNote(event.target.value)}
              maxLength={64}
              placeholder="版本说明（最多 64 字）"
              className="min-w-0 rounded-lg border border-slate-200 bg-transparent px-3 py-2 text-sm dark:border-slate-700"
            />
            <button
              className="btn"
              disabled={Boolean(pending)}
              onClick={() =>
                void run("draft", async () => {
                  await postJson("api/v1/config/draft", body());
                  await reload();
                  setInfo("共享草稿已保存，尚未激活。");
                })
              }
            >
              <Save size={14} />
              保存草稿
            </button>
            <button
              className="btn"
              disabled={Boolean(pending)}
              onClick={() => void run("validate", validate)}
            >
              <CheckCheck size={14} />
              校验并预览
            </button>
            <button
              className="btn"
              disabled={Boolean(pending) || !valid}
              onClick={() =>
                void run("revision", async () => {
                  const revision = await postJson<ConfigRevision>(
                    "api/v1/config/revisions",
                    body(),
                  );
                  setSaved(revision);
                  await reload();
                  setInfo(`版本 r${revision.id} 已保存，尚未激活。`);
                })
              }
            >
              保存版本
            </button>
            {saved && (
              <button
                className="btn"
                disabled={Boolean(pending)}
                onClick={() => void run("activate", () => activate(saved.id))}
              >
                <Play size={14} />
                激活 r{saved.id}
              </button>
            )}
            {pending && (
              <span className="muted text-xs" role="status">
                操作进行中…
              </span>
            )}
          </div>
        </div>
      </section>
      {validation && (
        <section className="panel mt-6 min-w-0 p-5">
          <div className="flex items-center gap-3">
            <h2 className="font-semibold">校验与差异</h2>
            <Badge active={validation.valid}>
              {validation.valid ? "通过" : "未通过"}
            </Badge>
          </div>
          {validation.diagnostics.map((issue, index) => (
            <p
              key={index}
              className="mt-3 text-sm text-red-700 dark:text-red-300"
            >
              {issue.path}
              {issue.line
                ? ` · 第 ${issue.line} 行，第 ${issue.column} 列`
                : ""}
              ：{issue.message}
            </p>
          ))}
          {validation.restart_required.length > 0 && (
            <p className="muted mt-3 text-sm">
              需重启：{validation.restart_required.join("、")}
            </p>
          )}
          {!validation.changes.length && validation.valid && (
            <p className="muted mt-3 text-sm">与实际配置没有字段差异。</p>
          )}
          <div className="mt-4 space-y-4">
            {validation.changes.map((change) => (
              <div
                key={change.path}
                className="rounded-xl border border-slate-200 p-4 dark:border-slate-800"
              >
                <p className="break-all font-mono text-xs">
                  {change.path} ·{" "}
                  {change.apply_mode === "restart" ? "需重启" : "在线激活"}
                </p>
                <div className="mt-3 grid gap-3 lg:grid-cols-2">
                  <pre className="max-h-52 overflow-auto rounded-lg bg-slate-50 p-3 text-xs dark:bg-slate-950">
                    {preview(change.before)}
                  </pre>
                  <pre className="max-h-52 overflow-auto rounded-lg bg-blue-50 p-3 text-xs dark:bg-blue-950">
                    {preview(change.after)}
                  </pre>
                </div>
              </div>
            ))}
          </div>
          {validation.changes.length > 0 && (
            <p className="muted mt-3 text-xs">
              每项差异最多展示 5,000 字符，完整内容见编辑器和脱敏导出。
            </p>
          )}
        </section>
      )}
      <section className="panel mt-6 min-w-0">
        <div className="border-b border-slate-100 p-5 dark:border-slate-800">
          <h2 className="font-semibold">配置版本</h2>
          <p className="muted mt-1 text-xs">
            最近 50 个保留版本；回滚仅允许已实际应用的版本，并重新验证兼容性。
          </p>
        </div>
        <div className="overflow-x-auto">
          <table className="w-full min-w-[680px] text-left text-sm">
            <thead className="muted border-b border-slate-100 text-xs dark:border-slate-800">
              <tr>
                <th className="p-4">版本</th>
                <th className="p-4">说明 / 时间</th>
                <th className="p-4">实际应用</th>
                <th className="p-4">操作</th>
              </tr>
            </thead>
            <tbody>
              {workspace.revisions.map((revision) => (
                <tr
                  key={revision.id}
                  className="border-b border-slate-100 last:border-0 dark:border-slate-800"
                >
                  <td className="p-4">
                    <span className="font-mono">r{revision.id}</span>
                    {revision.id === workspace.desired_revision && (
                      <span className="ml-2 text-xs text-blue-600">期望</span>
                    )}
                    {revision.id === workspace.running_revision && (
                      <span className="ml-2 text-xs text-emerald-600">
                        运行
                      </span>
                    )}
                  </td>
                  <td className="p-4">
                    <p>{revision.note}</p>
                    <p className="muted mt-1 text-xs">
                      {date(revision.created_at)}
                    </p>
                  </td>
                  <td className="p-4 text-xs">
                    {revision.applied_at
                      ? date(revision.applied_at)
                      : "尚未实际应用"}
                  </td>
                  <td className="p-4">
                    <div className="flex gap-2">
                      <button
                        className="btn"
                        disabled={Boolean(pending)}
                        onClick={() =>
                          void run("load", async () => {
                            const exported = await getJson<{ toml: string }>(
                              `api/v1/config/revisions/${revision.id}`,
                            );
                            setEditor(exported.toml);
                            setUseDraftCredentials(false);
                            setValidation(null);
                            setValidated(null);
                            setSaved(null);
                            setInfo(
                              `r${revision.id} 已载入编辑器，尚未修改运行配置。`,
                            );
                          })
                        }
                      >
                        载入
                      </button>
                      {revision.id !== workspace.desired_revision && (
                        <button
                          className="btn"
                          aria-label={`激活版本 ${revision.id}`}
                          disabled={Boolean(pending)}
                          onClick={() =>
                            void run("activate", () => activate(revision.id))
                          }
                        >
                          <Play size={13} />
                          激活
                        </button>
                      )}
                      {revision.applied_at &&
                        revision.id !== workspace.desired_revision && (
                          <button
                            className="btn"
                            aria-label={`回滚版本 ${revision.id}`}
                            disabled={Boolean(pending)}
                            onClick={() =>
                              void run("rollback", () =>
                                activate(revision.id, true),
                              )
                            }
                          >
                            <RotateCcw size={13} />
                            回滚
                          </button>
                        )}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
      <div className="mt-6 grid gap-6 lg:grid-cols-2">
        <section className="panel min-w-0">
          <h2 className="border-b border-slate-100 p-5 font-semibold dark:border-slate-800">
            存储与激活记录
          </h2>
          <Details
            rows={[
              ["数据存储", workspace.storage_path ?? "临时内存，退出后丢弃"],
              ["启动参数覆盖", workspace.startup_overrides.join("、") || "无"],
              ["共享草稿更新时间", date(workspace.draft.updated_at)],
            ]}
          />
          <div className="space-y-3 p-5">
            {workspace.activations.slice(0, 8).map((activation) => (
              <p key={activation.id} className="break-all text-xs leading-6">
                #{activation.id} · r{activation.previous_revision} → r
                {activation.revision} · {activation.state}
                {activation.error_code ? ` · ${activation.error_code}` : ""}
              </p>
            ))}
          </div>
        </section>
        <section className="panel p-5">
          <h2 className="font-semibold">激活影响</h2>
          <p className="muted mt-3 text-sm leading-7">
            代理入口、上游、资源限制和实例名称可在线激活。管理监听、管理 TLS
            和日志过滤需重启；期望版本会先保存，实际运行版本保持不变。监听或证书准备失败会保留旧运行代际。
          </p>
          <p className="muted mt-3 text-sm leading-7">
            已有连接保留原配置并排空，停止入口会终止该入口各代际的连接。激活过程由后台继续完成，离开页面后可通过记录确认结果。配置回滚只切换运行配置，不降级
            SQLite 表结构。
          </p>
        </section>
      </div>
    </>
  );
}
