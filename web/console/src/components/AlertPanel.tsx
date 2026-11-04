import { useEffect, useRef, useState, type FormEvent } from "react";
import { getJson, postJson } from "../platform";
type Settings = {
  enabled: boolean;
  no_exit: boolean;
  provider_failure: boolean;
  capture_failure: boolean;
  failure_rate: boolean;
  cooldown_secs: number;
  failure_min_count: number;
  failure_percent: number;
};
type Feed = {
  epoch: string;
  latest_sequence: number;
  truncated: boolean;
  retention_limit: number;
  running_revision: number;
  settings: Settings;
  notices: {
    sequence: number;
    occurred_at: number;
    key: string;
    state: string;
    code: string;
    message: string;
    suppressed_changes: number;
  }[];
};
const names: Record<string, string> = {
  enabled: "启用状态告警",
  no_exit: "无可用出口",
  provider_failure: "供应商提取失败",
  capture_failure: "Linux 捕获未就绪",
  failure_rate: "业务失败率异常",
};
export default function AlertPanel() {
  const [feed, setFeed] = useState<Feed>(),
    [settings, setSettings] = useState<Settings>(),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [info, setInfo] = useState("");
  const [refresh, setRefresh] = useState(0),
    [candidate, setCandidate] = useState<{ id: number; expected: number }>();
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    async function load() {
      try {
        if (document.hidden) return;
        const value = await getJson<Feed>(
          "api/v1/observability/alerts?limit=100",
          controller.signal,
        );
        if (!controller.signal.aborted) {
          setFeed(value);
          setSettings((v) => v ?? value.settings);
        }
      } catch (e) {
        if (!controller.signal.aborted) setError(String(e));
      } finally {
        if (!controller.signal.aborted)
          timer = setTimeout(() => void load(), 5000);
      }
    }
    void load();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [refresh]);
  async function save(e: FormEvent) {
    e.preventDefault();
    if (!feed || !settings || busy) return;
    setBusy(true);
    setError("");
    setInfo("");
    try {
      const saved = await postJson<{ id: number }>(
        "api/v1/observability/alerts/save",
        { expected_revision: feed.running_revision, settings },
      );
      if (mounted.current) {
        setCandidate({ id: saved.id, expected: feed.running_revision });
        setInfo(`告警设置候选 r${saved.id} 已保存；审阅后应用。`);
      }
    } catch (e) {
      if (mounted.current) setError(String(e));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  async function activate() {
    if (!candidate || busy) return;
    setBusy(true);
    setError("");
    try {
      const result = await postJson<{ state: string }>(
        "api/v1/config/activate",
        { revision: candidate.id, expected_revision: candidate.expected },
      );
      if (mounted.current) {
        setInfo(`告警设置激活结果：${result.state}`);
        setCandidate(undefined);
        setRefresh((n) => n + 1);
      }
    } catch (e) {
      if (mounted.current) setError(String(e));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }
  return (
    <section className="panel panel-body space-y-4" aria-label="状态告警">
      <h2 className="font-semibold">状态告警</h2>
      <p className="muted text-xs leading-6">
        每 5 秒读取实际状态，变化时通知并按条件冷却；保留最近 128
        条，服务重启后重建。失败率来自约一分钟实际关闭/失败连接，策略拒绝不计入。控制台与
        CLI 共用通知事实，Desktop 可订阅系统通知；没有账号未读状态。
      </p>
      {settings && (
        <form
          aria-label="告警设置"
          onSubmit={(e) => void save(e)}
          className="space-y-3"
        >
          <div className="flex flex-wrap gap-4">
            {Object.entries(names).map(([key, name]) => (
              <label key={key} className="flex items-center gap-2 text-xs">
                <input
                  type="checkbox"
                  checked={settings[key as keyof Settings] as boolean}
                  disabled={busy || !!candidate}
                  onChange={(e) =>
                    setSettings({ ...settings, [key]: e.target.checked })
                  }
                />
                {name}
              </label>
            ))}
          </div>
          <div className="grid gap-3 sm:grid-cols-3">
            {(
              [
                ["cooldown_secs", "冷却秒数", 5, 3600],
                ["failure_min_count", "失败率最少完成连接", 1, 100000],
                ["failure_percent", "失败率阈值 %", 1, 100],
              ] as const
            ).map(([key, name, min, max]) => (
              <label key={key} className="form-field">
                {name}
                <input
                  aria-label={name}
                  className="field"
                  type="number"
                  min={min}
                  max={max}
                  value={settings[key]}
                  disabled={busy || !!candidate}
                  onChange={(e) =>
                    setSettings({ ...settings, [key]: Number(e.target.value) })
                  }
                />
              </label>
            ))}
          </div>
          <div className="flex flex-wrap gap-2">
            <button className="btn" disabled={busy || !!candidate}>
              保存告警设置候选
            </button>
            <button
              type="button"
              className="btn"
              disabled={busy || !!candidate}
              onClick={() => {
                setSettings(undefined);
                setError("");
                setRefresh((n) => n + 1);
              }}
            >
              重新读取当前设置
            </button>
            {candidate && (
              <>
                <a
                  className="btn"
                  href={`./config.html?revision=${candidate.id}`}
                >
                  审阅配置 r{candidate.id}
                </a>
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={busy}
                  onClick={() => void activate()}
                >
                  应用告警设置
                </button>
              </>
            )}
          </div>
        </form>
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
      {feed && (
        <>
          <p className="muted break-all text-xs">
            观察实例 {feed.epoch} · 当前 r{feed.running_revision} · 通知序号{" "}
            {feed.latest_sequence}
            {feed.truncated ? " · 部分旧通知已超过保留窗口" : ""}
          </p>
          <ol className="space-y-2">
            {feed.notices
              .slice()
              .reverse()
              .map((n) => (
                <li
                  key={`${feed.epoch}:${n.sequence}`}
                  className="rounded-xl bg-console-soft p-3 text-xs"
                >
                  <p className="font-medium">
                    {n.state === "recovered" ? "已恢复" : "异常"} · {n.key} ·{" "}
                    {new Date(n.occurred_at * 1000).toLocaleString()}
                  </p>
                  <p className="mt-1 break-words">{n.message}</p>
                  <p className="muted mt-1">
                    {n.code} · #{n.sequence}
                    {n.suppressed_changes
                      ? ` · 冷却合并 ${n.suppressed_changes} 次变化`
                      : ""}
                  </p>
                </li>
              ))}
          </ol>
          {!feed.notices.length && (
            <p className="muted text-xs">当前观察实例尚无状态变化通知。</p>
          )}
        </>
      )}
    </section>
  );
}
