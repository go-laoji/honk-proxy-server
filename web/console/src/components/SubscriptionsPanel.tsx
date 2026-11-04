import { useEffect, useRef, useState } from "react";
import {
  subscriptionApi,
  type SubscriptionConfig,
  type SubscriptionsOverview,
} from "../subscriptions";
const blank: SubscriptionConfig = {
  id: "standard",
  name: "标准订阅",
  enabled: true,
  url: "https://example.com/subscription",
  format: "uri",
  timeout_ms: 10000,
  max_nodes: 1000,
  snapshot: null,
};
export default function SubscriptionsPanel({
  refreshService,
  onRevision,
}: {
  refreshService: () => Promise<void>;
  onRevision: () => Promise<void>;
}) {
  const [data, setData] = useState<SubscriptionsOverview>();
  const [error, setError] = useState("");
  const [info, setInfo] = useState("");
  const [busy, setBusy] = useState(false);
  const [editor, setEditor] = useState<{
    body: string;
    revision: number;
  } | null>(null);
  const [importing, setImporting] = useState<{
    id: string;
    revision: number;
    document: string;
  } | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    const controller = new AbortController();
    subscriptionApi
      .list(controller.signal)
      .then((value) => {
        if (alive.current) setData(value);
      })
      .catch((cause) => {
        if (alive.current && !controller.signal.aborted)
          setError(String(cause));
      });
    return () => {
      alive.current = false;
      controller.abort();
    };
  }, []);
  async function reload() {
    const value = await subscriptionApi.list();
    if (alive.current) setData(value);
  }
  async function action(work: () => Promise<unknown>) {
    setBusy(true);
    setError("");
    setInfo("");
    try {
      await work();
      if (!alive.current) return;
      await Promise.all([reload(), refreshService(), onRevision()]);
      if (alive.current) {
        setEditor(null);
        setImporting(null);
        setInfo("订阅配置与快照已按版本应用。已有连接继续由原代际拥有。");
      }
    } catch (cause) {
      if (alive.current)
        setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  return (
    <section className="panel mb-6 space-y-4 p-5" aria-label="标准订阅">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-semibold">标准订阅 · 独立于短效提取 API</h2>
        <div className="flex gap-2">
          <button
            className="btn"
            disabled={busy}
            onClick={() =>
              void reload().catch((cause) => setError(String(cause)))
            }
          >
            刷新订阅视图
          </button>
          <button
            className="btn-primary"
            disabled={busy || !data}
            onClick={() =>
              setEditor({
                body: JSON.stringify(blank, null, 2),
                revision: data!.running_revision,
              })
            }
          >
            新增标准订阅
          </button>
        </div>
      </div>
      <p className="muted text-xs leading-6">
        支持 HTTP/HTTPS/SOCKS5 URI、整文 base64 URI 列表及 Clash proxies
        中对应类型。其他协议或不支持字段会明确拒绝整份刷新。快照接收时间不是供应商有效期；标准订阅有效期未知，不创建短效租约或触发补池购买。远程刷新仅执行一次
        GET，失败保留旧快照。
      </p>
      {error && (
        <p role="alert" className="text-sm text-red-600">
          {error}
        </p>
      )}
      {info && (
        <p role="status" className="text-sm text-blue-600">
          {info}
        </p>
      )}
      {data?.subscriptions.length === 0 && (
        <p className="muted text-sm">尚无标准订阅；短效供应商仍在下方管理。</p>
      )}
      <div className="grid gap-4 lg:grid-cols-2">
        {data?.subscriptions.map((source) => (
          <article
            className="rounded-xl border border-slate-200 p-4 dark:border-slate-700"
            key={source.id}
          >
            <h3 className="break-all font-medium">
              {source.name || source.id} · {source.id}
            </h3>
            <p className="muted mt-2 text-xs leading-6">
              {source.enabled ? "启用" : "停用"} · {source.format.toUpperCase()}{" "}
              · {source.snapshot?.nodes.length ?? 0} 个快照节点 · 有效期未知
            </p>
            <p className="muted text-xs">
              {source.snapshot
                ? `接收 ${new Date(source.snapshot.fetched_at_unix * 1000).toLocaleString("zh-CN")} · ${source.snapshot.origin === "remote_get" ? "远程 GET" : "文档导入"}`
                : "尚无成功快照"}{" "}
              · URL 已脱敏
            </p>
            <p className="muted mt-2 break-all text-xs">
              分组来源：subscription:{source.id}
            </p>
            <div className="mt-3 flex flex-wrap gap-2">
              <button
                className="btn"
                disabled={busy || !source.enabled}
                onClick={() =>
                  void action(() =>
                    subscriptionApi.refresh(
                      source.id,
                      data.running_revision,
                      null,
                    ),
                  )
                }
              >
                GET 更新订阅
              </button>
              <button
                className="btn"
                disabled={busy || !source.enabled}
                onClick={() =>
                  setImporting({
                    id: source.id,
                    revision: data.running_revision,
                    document: "",
                  })
                }
              >
                导入订阅文档
              </button>
              <button
                className="btn"
                disabled={busy}
                onClick={() =>
                  setEditor({
                    body: JSON.stringify(
                      { ...source, snapshot: null },
                      null,
                      2,
                    ),
                    revision: data.running_revision,
                  })
                }
              >
                编辑订阅
              </button>
              <button
                className="btn"
                disabled={busy}
                onClick={() =>
                  void action(() =>
                    subscriptionApi.save(
                      { ...source, enabled: !source.enabled },
                      data.running_revision,
                    ),
                  )
                }
              >
                {source.enabled ? "停用订阅" : "启用订阅"}
              </button>
              <button
                className="btn text-red-600"
                disabled={busy}
                onClick={() =>
                  void action(() =>
                    subscriptionApi.delete(source.id, data.running_revision),
                  )
                }
              >
                删除订阅
              </button>
            </div>
            <a
              className="mt-3 inline-block text-xs text-blue-600"
              href={`./nodes.html?provider=${encodeURIComponent(`subscription:${source.id}`)}`}
            >
              查看来源节点
            </a>
          </article>
        ))}
      </div>
      {editor && (
        <div className="space-y-3" aria-label="订阅编辑器">
          <label className="form-field">
            订阅定义 JSON · r{editor.revision}
            <textarea
              className="field editor-field min-h-64"
              spellCheck={false}
              value={editor.body}
              onChange={(event) =>
                setEditor({ ...editor, body: event.target.value })
              }
            />
          </label>
          <p className="muted text-xs">
            保留 [REDACTED] 可复用已有 URL；新 URL 不会明文回显到视图。快照由
            GET/导入管理。
          </p>
          <button
            className="btn-primary"
            disabled={busy}
            onClick={() =>
              void action(() =>
                subscriptionApi.save(
                  JSON.parse(editor.body) as SubscriptionConfig,
                  editor.revision,
                ),
              )
            }
          >
            保存并应用订阅
          </button>
          <button
            className="btn ml-2"
            disabled={busy}
            onClick={() => setEditor(null)}
          >
            取消订阅编辑
          </button>
        </div>
      )}
      {importing && (
        <div className="space-y-3" aria-label="导入订阅文档">
          <label className="form-field">
            {importing.id} · 文档内容 · r{importing.revision}
            <textarea
              className="field editor-field min-h-48"
              spellCheck={false}
              value={importing.document}
              onChange={(event) =>
                setImporting({ ...importing, document: event.target.value })
              }
            />
          </label>
          <button
            className="btn-primary"
            disabled={busy || !importing.document.trim()}
            onClick={() =>
              void action(() =>
                subscriptionApi.refresh(
                  importing.id,
                  importing.revision,
                  importing.document,
                ),
              )
            }
          >
            解析并替换该订阅快照
          </button>
          <button
            className="btn ml-2"
            disabled={busy}
            onClick={() => setImporting(null)}
          >
            取消文档导入
          </button>
        </div>
      )}
    </section>
  );
}
