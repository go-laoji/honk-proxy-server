import { useEffect, useRef, useState } from "react";
import { Play, Square, RefreshCw, Save } from "lucide-react";
import { Badge, Details } from "../components/ui";
import { getJson, postJson } from "../platform";
import {
  defaultCapture,
  type CaptureOverview,
  type CaptureStatus,
  type LinuxCaptureConfig,
} from "../capture";
import type { ConfigRevision } from "../types";
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const words = (value: string) => value.split(/[\s,]+/).filter(Boolean);
const states: Record<string, string> = {
  running: "运行中",
  stopped: "已停止",
  degraded: "已降级",
  busy: "操作中",
  unconfigured: "未配置",
  unsupported: "平台不支持",
};
const fact = (value: boolean | null) =>
  value === null ? "未核验" : value ? "已核验" : "缺失";
export default function CapturePanel({
  refreshService,
}: {
  refreshService: () => Promise<void>;
}) {
  const [view, setView] = useState<CaptureOverview | null>(null);
  const [editor, setEditor] = useState<LinuxCaptureConfig | null>(null);
  const [editorRevision, setEditorRevision] = useState<number | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [saved, setSaved] = useState<ConfigRevision | null>(null);
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [pending, setPending] = useState("");
  const [attempt, setAttempt] = useState(0);
  const mounted = useRef(true);
  const initialized = useRef(false);
  useEffect(() => {
    mounted.current = true;
    const abort = new AbortController();
    void getJson<CaptureOverview>("api/v1/linux/capture", abort.signal)
      .then((value) => {
        if (abort.signal.aborted) return;
        setView(value);
        setLoadError("");
        if (!initialized.current) {
          setEditor(value.desired_configuration ?? defaultCapture());
          setEditorRevision(value.desired_revision);
          setEnabled(value.desired_configuration !== null);
          initialized.current = true;
        }
      })
      .catch((e) => {
        if (!abort.signal.aborted) setLoadError(message(e));
      });
    return () => {
      mounted.current = false;
      abort.abort();
    };
  }, [attempt]);
  async function run(label: string, work: () => Promise<void>) {
    setPending(label);
    setError("");
    try {
      await work();
    } catch (e) {
      if (mounted.current) setError(message(e));
    } finally {
      if (mounted.current) {
        setPending("");
        setAttempt((a) => a + 1);
      }
    }
  }
  function control(action: "start" | "stop" | "recheck") {
    return run(action, async () => {
      const status = await postJson<CaptureStatus>(
        "api/v1/linux/capture/control",
        {
          action,
          expected_epoch: view!.epoch,
          expected_revision: view!.running_revision,
        },
      );
      if (mounted.current) setView((v) => (v ? { ...v, status } : v));
      if (mounted.current) await refreshService();
    });
  }
  const status = view?.status;
  const supported = status?.platform === "linux";
  const editable =
    !!view &&
    supported &&
    view.running_revision === view.desired_revision &&
    editorRevision === view.desired_revision;
  return (
    <section className="panel overflow-hidden" aria-label="透明代理运行控制">
      <div className="panel-header panel-toolbar">
        <div>
          <h2 className="font-semibold">透明代理运行控制</h2>
          <p className="muted mt-2 text-xs">
            实际运行对象、捕获范围与故障恢复状态
          </p>
        </div>
        <button
          className="btn"
          disabled={!!pending}
          onClick={() => setAttempt((a) => a + 1)}
        >
          <RefreshCw size={15} />
          刷新运行状态
        </button>
      </div>
      {(error || loadError) && (
        <p role="alert" className="break-words p-5 text-sm text-console-danger">
          {error || loadError}
        </p>
      )}
      {status ? (
        <>
          <div className="flex flex-wrap items-center gap-3 px-5 pt-5">
            <Badge active={status.ready}>
              {states[status.state] ?? status.state}
            </Badge>
            <span className="muted text-xs">
              {status.suspended
                ? "故障保护已暂停新流量"
                : status.ready
                  ? "已接管所选范围"
                  : "尚未就绪"}
            </span>
          </div>
          {status.reason && (
            <p className="muted break-words px-5 pt-4 text-sm">
              {status.reason}
            </p>
          )}
          <Details
            rows={[
              [
                "运行配置 / 期望配置",
                `r${view!.running_revision} / r${view!.desired_revision}`,
              ],
              [
                "用户态 / 内核代际",
                `r${status.userspace_revision} / ${status.kernel_revision === null ? "未发布" : `r${status.kernel_revision}`}`,
              ],
              [
                "捕获链接 / 自有路由",
                `${fact(status.links_present)} / ${fact(status.routes_present)}`,
              ],
              [
                "捕获网卡启用与链路状态",
                status.interfaces_ready === true
                  ? "可用"
                  : status.interfaces_ready === false
                    ? "不可用"
                    : "未核验",
              ],
              ["本机 cgroup", status.scope?.local_cgroup ?? "未选择本机范围"],
              [
                "LAN 网卡",
                status.scope?.lan_interfaces.join(", ") || "未选择网关范围",
              ],
              [
                "透明监听",
                status.listeners
                  .map((s) => `${s.transport} ${s.address}`)
                  .join(" · ") || "无",
              ],
              [
                "内核流 / 活跃连接 / 入口拒绝",
                `${status.flow_entries} / ${status.active_owners} / ${status.rejected_ingress}`,
              ],
              [
                "自动恢复尝试",
                `${status.retry_attempts} / ${status.max_retry_attempts}`,
              ],
              [
                "检查时间",
                new Date(status.checked_at_unix_ms).toLocaleString("zh-CN"),
              ],
            ]}
          />
          <div className="flex flex-wrap gap-3 px-5 pb-5">
            <button
              className="btn"
              disabled={
                !!pending || !supported || !status.configured || status.ready
              }
              onClick={() => void control("start")}
            >
              <Play size={15} />
              启动透明代理
            </button>
            <button
              className="btn"
              disabled={
                !!pending ||
                !supported ||
                !status.configured ||
                status.state === "stopped"
              }
              onClick={() => void control("stop")}
            >
              <Square size={15} />
              停止透明代理
            </button>
            <button
              className="btn"
              disabled={!!pending || !supported || !status.configured}
              onClick={() => void control("recheck")}
            >
              <RefreshCw size={15} />
              重新检查并恢复
            </button>
            <a className="btn" href="./tasks.html?kind=linux_capture_control">
              查看操作任务
            </a>
          </div>
          <details className="border-t border-console-border p-5">
            <summary className="cursor-pointer text-sm">
              内核对象与资源证据
            </summary>
            <pre className="mt-4 max-h-80 overflow-auto whitespace-pre-wrap break-all text-xs">
              {JSON.stringify(
                {
                  object: status.object_path,
                  fingerprint: status.object_fingerprint,
                  programs: status.program_ids,
                  maps: status.map_ids,
                  generations: status.generations,
                  resources: status.resources,
                  hooks: status.hooks,
                },
                null,
                2,
              )}
            </pre>
          </details>
        </>
      ) : (
        <p className="muted p-5">正在读取运行状态…</p>
      )}
      {editor && (
        <form
          className="space-y-4 border-t border-console-border p-5"
          onSubmit={(event) => {
            event.preventDefault();
            void run("save", async () => {
              const result = await postJson<ConfigRevision>(
                "api/v1/linux/capture/save",
                {
                  expected_epoch: view!.epoch,
                  expected_revision: editorRevision,
                  configuration: enabled ? editor : null,
                },
              );
              if (mounted.current) setSaved(result);
            });
          }}
        >
          <h3 className="font-semibold">捕获配置候选</h3>
          <p className="muted text-xs leading-6">
            配置在目标 Linux Daemon
            上生效。保存后到配置工作区审阅并激活；修改范围、对象、端口或预算需要重启。停止按钮仅停止当前透明代理，持久关闭请保存禁用候选。
          </p>
          <fieldset className="space-y-4" disabled={!!pending || !editable}>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => setEnabled(e.target.checked)}
              />
              持久启用透明捕获
            </label>
            <div className="grid gap-4 sm:grid-cols-2">
              <label className="form-field">
                eBPF 对象路径
                <input
                  className="field"
                  value={editor.object}
                  onChange={(e) =>
                    setEditor({ ...editor, object: e.target.value })
                  }
                />
              </label>
              <label className="form-field">
                本机 cgroup 路径
                <input
                  className="field"
                  value={editor.local_cgroup ?? ""}
                  onChange={(e) =>
                    setEditor({
                      ...editor,
                      local_cgroup: e.target.value || null,
                    })
                  }
                />
              </label>
              <label className="form-field">
                LAN 网卡（逗号分隔）
                <input
                  className="field"
                  value={editor.lan_interfaces.join(",")}
                  onChange={(e) =>
                    setEditor({
                      ...editor,
                      lan_interfaces: e.target.value.split(","),
                    })
                  }
                  onBlur={() =>
                    setEditor({
                      ...editor,
                      lan_interfaces: words(editor.lan_interfaces.join(",")),
                    })
                  }
                />
              </label>
              <label className="form-field">
                旁路 CIDR（逗号分隔）
                <input
                  className="field"
                  value={editor.bypass_cidrs.join(",")}
                  onChange={(e) =>
                    setEditor({
                      ...editor,
                      bypass_cidrs: e.target.value.split(","),
                    })
                  }
                  onBlur={() =>
                    setEditor({
                      ...editor,
                      bypass_cidrs: words(editor.bypass_cidrs.join(",")),
                    })
                  }
                />
              </label>
            </div>
            <div className="flex flex-wrap gap-4">
              {(["ipv4", "ipv6", "tcp", "udp"] as const).map((key) => (
                <label className="flex items-center gap-2 text-sm" key={key}>
                  <input
                    type="checkbox"
                    checked={editor[key]}
                    onChange={(e) =>
                      setEditor({ ...editor, [key]: e.target.checked })
                    }
                  />
                  {key.toUpperCase()}
                </label>
              ))}
            </div>
            <details>
              <summary className="cursor-pointer text-sm">
                路由、端口与容量
              </summary>
              <div className="mt-4 grid gap-3 sm:grid-cols-3">
                {(
                  [
                    "capture_mark",
                    "bypass_mark",
                    "routing_table",
                    "rule_priority",
                    "tcp_port",
                    "udp_port",
                    "max_flows",
                    "max_pending",
                    "max_udp_sessions",
                    "udp_idle_secs",
                    "max_datagram_bytes",
                    "max_queued_datagrams",
                    "max_queued_bytes",
                  ] as const
                ).map((key) => (
                  <label className="form-field break-all" key={key}>
                    {key}
                    <input
                      className="field"
                      type="number"
                      min="0"
                      value={editor[key]}
                      onChange={(e) =>
                        setEditor({ ...editor, [key]: Number(e.target.value) })
                      }
                    />
                  </label>
                ))}
              </div>
            </details>
            <button className="btn" type="submit">
              <Save size={15} />
              {pending === "save" ? "正在保存…" : "保存捕获配置候选"}
            </button>
          </fieldset>
          {supported && view && editorRevision !== view.desired_revision && (
            <p role="status" className="text-sm text-console-warning">
              配置已变化，当前编辑内容未覆盖。请载入最新捕获配置后编辑。
            </p>
          )}
          <button
            className="btn"
            type="button"
            disabled={!!pending || !supported}
            onClick={() => {
              initialized.current = false;
              setError("");
              setSaved(null);
              setAttempt((a) => a + 1);
            }}
          >
            载入最新捕获配置
          </button>
          {!supported && (
            <p className="muted text-xs">
              当前服务平台不支持 eBPF。可连接 Linux Daemon 后管理。
            </p>
          )}
          {view && view.running_revision !== view.desired_revision && (
            <p className="muted text-xs">
              存在尚未应用的期望配置，请先在配置工作区处理。
            </p>
          )}
          {saved && (
            <p role="status" className="text-sm">
              已保存候选 r{saved.id}，
              <a
                className="text-console-accent-ink hover:underline"
                href={`./config.html?revision=${saved.id}&epoch=${encodeURIComponent(view!.epoch)}`}
              >
                审阅并激活配置
              </a>
              。
            </p>
          )}
          <a
            className="text-sm text-console-accent-ink hover:underline"
            href="./config.html"
          >
            打开配置工作区
          </a>
        </form>
      )}
    </section>
  );
}
