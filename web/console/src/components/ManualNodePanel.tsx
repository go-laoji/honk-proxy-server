import { useRef, useState, type FormEvent } from "react";
import { Eye, EyeOff, Plus, X } from "lucide-react";
import { ApiRequestError, postJson } from "../platform";
import type { ImportPreview, ImportRequest } from "../nodes";
import {
  defaultPort,
  manualNodeDocument,
  readProxyUrl,
  type ManualNode,
  type ManualProtocol,
} from "../manual-node";

export default function ManualNodePanel({
  revision,
  busy,
  onBusy,
  onRefresh,
  onAdded,
  onClose,
}: {
  revision: number | null;
  busy: boolean;
  onBusy: (value: boolean) => void;
  onRefresh: () => void;
  onAdded: (id: string) => void;
  onClose: () => void;
}) {
  const [generatedId] = useState(
    () => `manual-${crypto.getRandomValues(new Uint32Array(2)).join("-")}`,
  );
  const [form, setForm] = useState<ManualNode>({
    id: "",
    protocol: "http",
    host: "",
    port: "80",
    username: "",
    password: "",
    enabled: true,
    expires: "",
  });
  const [url, setUrl] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [error, setError] = useState("");
  const pending = useRef(false);
  function change<K extends keyof ManualNode>(key: K, value: ManualNode[K]) {
    setForm((current) => ({ ...current, [key]: value }));
    setError("");
  }
  function fillUrl() {
    try {
      const fields = readProxyUrl(url);
      setForm((current) => ({ ...current, ...fields }));
      setUrl("");
      setError("");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "无法读取代理链接");
    }
  }
  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy || pending.current || revision === null) return;
    pending.current = true;
    onBusy(true);
    setError("");
    try {
      const { id, document } = manualNodeDocument(form, generatedId);
      const input: ImportRequest = {
        document,
        format: "json",
        default_protocol: form.protocol,
        expected_revision: revision,
        preview_checksum: null,
        selected_rows: [],
      };
      const preview = await postJson<ImportPreview>(
        "api/v1/nodes/import/preview",
        input,
      );
      if (!preview.applicable) {
        const code = preview.rows.find((r) => !r.valid)?.error_code;
        throw new Error(
          code === "duplicate_node_id"
            ? "节点标识已存在，请更换标识；现有节点未被覆盖。"
            : "节点配置无效，请检查地址、端口及上游认证信息。",
        );
      }
      await postJson("api/v1/nodes/import/apply", {
        ...input,
        preview_checksum: preview.checksum,
      });
      onAdded(id);
    } catch (cause) {
      if (cause instanceof ApiRequestError && cause.status === 409) {
        setError(
          "节点配置已被其他操作更新，已刷新版本；填写内容已保留，请重新添加。",
        );
        onRefresh();
      } else {
        setError(cause instanceof Error ? cause.message : "添加失败，请重试");
      }
    } finally {
      pending.current = false;
      onBusy(false);
    }
  }
  return (
    <section className="panel panel-body space-y-5" aria-label="手动添加节点">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="font-semibold">手动添加代理节点</h2>
          <p className="muted mt-2 text-xs leading-6">
            节点会保存在配置中，重启后保留。添加后可查看健康检测结果，并按分组规则参与选点。
          </p>
        </div>
        <button
          className="btn shrink-0"
          type="button"
          disabled={busy}
          onClick={onClose}
          aria-label="关闭手动添加"
        >
          <X size={16} />
        </button>
      </div>
      <div className="form-field rounded-xl border border-console-border p-4">
        <label htmlFor="manual-proxy-url">代理链接（可选，用于快速填写）</label>
        <div className="flex flex-col gap-2 sm:flex-row">
          <input
            id="manual-proxy-url"
            autoFocus
            className="field flex-1"
            type="password"
            autoComplete="off"
            spellCheck={false}
            disabled={busy}
            value={url}
            placeholder="socks5://用户名:密码@IP:端口"
            onChange={(e) => setUrl(e.target.value)}
          />
          <button
            className="btn shrink-0"
            type="button"
            disabled={busy || !url.trim()}
            onClick={fillUrl}
          >
            填入表单
          </button>
        </div>
        <p className="muted font-normal">
          支持 HTTP、HTTPS、SOCKS5 / SOCKS5h；链接和密码默认隐藏。
        </p>
      </div>
      <form
        onSubmit={(event) => void submit(event)}
        aria-label="单个代理节点"
        className="space-y-4"
      >
        <fieldset disabled={busy} className="grid min-w-0 gap-4 sm:grid-cols-2">
          <div className="form-field">
            <label htmlFor="manual-node-id">节点标识（可选）</label>
            <input
              id="manual-node-id"
              className="field"
              aria-describedby="manual-node-id-hint"
              value={form.id}
              maxLength={64}
              placeholder="留空自动生成，如 office-proxy"
              onChange={(e) => change("id", e.target.value)}
            />
            <span id="manual-node-id-hint" className="muted font-normal">
              自定义标识支持字母、数字、下划线和连字符。
            </span>
          </div>
          <label className="form-field">
            代理协议
            <select
              className="field"
              value={form.protocol}
              onChange={(e) => {
                const protocol = e.target.value as ManualProtocol;
                setForm((current) => ({
                  ...current,
                  protocol,
                  port:
                    current.port === defaultPort(current.protocol)
                      ? defaultPort(protocol)
                      : current.port,
                }));
                setError("");
              }}
            >
              <option value="http">HTTP</option>
              <option value="https">HTTPS</option>
              <option value="socks5">SOCKS5</option>
            </select>
          </label>
          <label className="form-field">
            IP 或域名
            <input
              className="field"
              value={form.host}
              required
              autoComplete="off"
              spellCheck={false}
              placeholder="如 127.0.0.1 或 ::1"
              onChange={(e) => change("host", e.target.value)}
            />
          </label>
          <label className="form-field">
            代理端口
            <input
              className="field"
              type="number"
              min={1}
              max={65535}
              step={1}
              required
              value={form.port}
              onChange={(e) => change("port", e.target.value)}
            />
          </label>
          <label className="form-field">
            上游用户名（可选）
            <input
              className="field"
              autoComplete="off"
              spellCheck={false}
              value={form.username}
              onChange={(e) => change("username", e.target.value)}
            />
          </label>
          <div className="form-field">
            <label htmlFor="manual-proxy-password">上游密码（可选）</label>
            <div className="flex gap-2">
              <input
                id="manual-proxy-password"
                className="field flex-1"
                type={showPassword ? "text" : "password"}
                autoComplete="new-password"
                value={form.password}
                onChange={(e) => change("password", e.target.value)}
              />
              <button
                className="btn shrink-0"
                type="button"
                aria-label={showPassword ? "隐藏上游密码" : "显示上游密码"}
                aria-pressed={showPassword}
                onClick={() => setShowPassword((v) => !v)}
              >
                {showPassword ? <EyeOff size={16} /> : <Eye size={16} />}
              </button>
            </div>
          </div>
          <div className="form-field">
            <label htmlFor="manual-node-expiry">
              到期时间（可选，本地时间）
            </label>
            <input
              id="manual-node-expiry"
              className="field"
              aria-describedby="manual-node-expiry-hint"
              type="datetime-local"
              value={form.expires}
              onChange={(e) => change("expires", e.target.value)}
            />
            <span id="manual-node-expiry-hint" className="muted font-normal">
              长效节点可留空；短效节点请填写供应商实际到期时间。
            </span>
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={form.enabled}
              onChange={(e) => change("enabled", e.target.checked)}
            />
            添加后启用
          </label>
        </fieldset>
        {error && (
          <p role="alert" className="break-words text-sm text-red-600">
            {error}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <button
            className="btn-primary"
            type="submit"
            disabled={busy || revision === null}
          >
            <Plus size={16} />
            {busy ? "正在添加…" : "添加节点"}
          </button>
          <button
            className="btn"
            type="button"
            disabled={busy}
            onClick={onClose}
          >
            取消
          </button>
        </div>
      </form>
    </section>
  );
}
