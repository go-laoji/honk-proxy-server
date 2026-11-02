import { useCallback, useEffect, useState } from "react";
import { native } from "../platform";
import type { LocalContext } from "../local-system";
import type { ServiceData } from "../use-service";
import { Badge } from "./ui";

export default function LocalSystemPanel({
  data,
  refreshService,
}: {
  data: ServiceData | null;
  refreshService: () => Promise<void>;
}) {
  const [context, setContext] = useState<LocalContext | null>(null);
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [busy, setBusy] = useState(false);
  const [binary, setBinary] = useState("");
  const [config, setConfig] = useState("");
  const [network, setNetwork] = useState("");
  const [http, setHttp] = useState("");
  const [socks, setSocks] = useState("");
  const refresh = useCallback(async () => {
    try {
      setContext(await native<LocalContext>("local_context"));
      setLoadError("");
    } catch (e) {
      setLoadError(String(e));
    }
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  async function act(command: string, args?: Record<string, unknown>) {
    if (!context?.local || !context.profile_id) return;
    setError("");
    setBusy(true);
    try {
      await native(command, {
        expected: {
          endpoint: context.endpoint,
          profile_id: context.profile_id,
        },
        ...args,
      });
      await refresh();
      await refreshService();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  const enabled = Boolean(context?.local && context.profile_id);
  const inbounds =
    data && data.endpoint === context?.endpoint
      ? data.proxy.inbounds.filter(
          (i) =>
            i.running &&
            i.listen &&
            /^(127\.[0-9.]+|0\.0\.0\.0):[0-9]+$/.test(i.listen),
        )
      : [];
  return (
    <section className="panel mt-6 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="font-semibold">本机服务与系统代理</h2>
        <button className="btn" disabled={busy} onClick={() => void refresh()}>
          重新检查本机
        </button>
      </div>
      {loadError && (
        <p role="alert" className="mt-3 text-sm text-red-600">
          {loadError}
        </p>
      )}
      {!enabled ? (
        <p className="muted mt-4 text-sm leading-7">
          先在连接档案中明确选择本机服务。当前目标为远端或临时地址，所有本机操作均已停用。
        </p>
      ) : (
        <>
          <p className="muted mt-3 break-all text-xs leading-6">
            本机档案 {context?.profile_id} · {context?.endpoint} ·{" "}
            {context?.platform}
            。窗口关闭后服务继续由操作系统管理。服务进程存活与
            API/代理就绪分别检查。
          </p>
          <div className="mt-4 flex flex-wrap items-center gap-3">
            <Badge active={Boolean(context?.service?.pid)}>
              服务：{context?.service?.state ?? "无法读取"}
            </Badge>
            <span className="muted text-xs">
              {context?.service?.manager}{" "}
              {context?.service?.pid ? `PID ${context.service.pid}` : ""}
            </span>
            <Badge active={Boolean(data?.status.management_ready)}>
              API {data ? "已连接" : "未连接"}
            </Badge>
          </div>
          {context?.service_error && (
            <p role="alert" className="mt-3 text-sm text-red-600">
              {context.service_error}
            </p>
          )}
          {!context?.service?.installed ? (
            <form
              className="mt-5 grid gap-3"
              onSubmit={(e) => {
                e.preventDefault();
                void act("local_service_install", { binary, config });
              }}
            >
              <label className="text-xs">
                hpsd 程序绝对路径
                <input
                  className="field mt-1"
                  required
                  value={binary}
                  onChange={(e) => setBinary(e.target.value)}
                />
              </label>
              <label className="text-xs">
                Daemon 配置绝对路径
                <input
                  className="field mt-1"
                  required
                  value={config}
                  onChange={(e) => setConfig(e.target.value)}
                />
              </label>
              <p className="muted text-xs leading-6">
                安装时会将程序和配置复制到持久目录；不会因为打开窗口而自动启动代理。Linux
                eBPF 请使用管理员 CLI 安装 system 范围服务，Desktop
                保持普通用户运行。
              </p>
              <button
                className="btn-primary w-fit"
                disabled={busy}
                type="submit"
              >
                安装本机用户服务
              </button>
            </form>
          ) : (
            <div className="mt-4 flex flex-wrap gap-2">
              {(
                [
                  ["start", "启动本机服务"],
                  ["stop", "停止本机服务"],
                  ["restart", "重启本机服务"],
                  ["uninstall", "卸载服务并保留数据"],
                ] as const
              ).map(([action, label]) => (
                <button
                  key={action}
                  className="btn"
                  disabled={
                    busy ||
                    context?.service?.manifest?.endpoint !==
                      context?.endpoint ||
                    context?.service?.manifest?.scope === "system"
                  }
                  onClick={() => void act("local_service_control", { action })}
                >
                  {label}
                </button>
              ))}
            </div>
          )}
          <div className="mt-6 border-t border-slate-100 pt-5 dark:border-slate-800">
            <h3 className="text-sm font-medium">系统代理原值恢复</h3>
            {context?.network_changes_disabled && (
              <p role="status" className="mt-2 text-sm text-amber-700">
                开发验收保护已开启：当前应用只读检查系统代理，修改与恢复均已停用。
              </p>
            )}
            <p className="muted mt-2 text-xs leading-6">
              {context?.proxy?.supported
                ? `当前状态：${context.proxy.state}。修改前保存完整原值；后续有外部修改时拒绝覆盖并保留备份。系统可能要求操作系统管理员授权。`
                : "当前平台未提供系统代理后端，请在浏览器或操作系统中手动指定代理入口。"}
            </p>
            {context?.proxy?.supported && (
              <form
                className="mt-4 grid gap-3"
                onSubmit={(e) => {
                  e.preventDefault();
                  void act("local_proxy_apply", {
                    input: {
                      network_service: network,
                      http: http ? Number(http) : null,
                      socks5: socks ? Number(socks) : null,
                    },
                  });
                }}
              >
                <label className="text-xs">
                  网络服务
                  <select
                    className="field mt-1"
                    required
                    value={network}
                    onChange={(e) => setNetwork(e.target.value)}
                  >
                    <option value="">请选择需要修改的网络服务</option>
                    {context.network_services.map((s) => (
                      <option key={s.id} value={s.id}>
                        {s.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="text-xs">
                  HTTP / HTTPS 目标代理入口
                  <select
                    className="field mt-1"
                    value={http}
                    onChange={(e) => setHttp(e.target.value)}
                  >
                    <option value="">不设置 HTTP 入口</option>
                    {inbounds
                      .filter((i) => i.protocol === "http")
                      .map((i) => (
                        <option key={i.id} value={i.listen!.split(":")[1]}>
                          {i.id} · {i.listen}
                        </option>
                      ))}
                  </select>
                </label>
                <label className="text-xs">
                  SOCKS5 入口
                  <select
                    className="field mt-1"
                    value={socks}
                    onChange={(e) => setSocks(e.target.value)}
                  >
                    <option value="">不设置 SOCKS5 入口</option>
                    {inbounds
                      .filter((i) => i.protocol === "socks5")
                      .map((i) => (
                        <option key={i.id} value={i.listen!.split(":")[1]}>
                          {i.id} · {i.listen}
                        </option>
                      ))}
                  </select>
                </label>
                <div className="flex flex-wrap gap-2">
                  <button
                    className="btn-primary"
                    type="submit"
                    disabled={
                      busy ||
                      context.network_changes_disabled ||
                      (!http && !socks) ||
                      context.proxy.state !== "unmanaged"
                    }
                  >
                    应用本机系统代理
                  </button>
                  <button
                    className="btn"
                    type="button"
                    disabled={
                      busy ||
                      context.network_changes_disabled ||
                      context.proxy.state === "unmanaged"
                    }
                    onClick={() => void act("local_proxy_restore")}
                  >
                    恢复原系统代理
                  </button>
                </div>
              </form>
            )}
          </div>
        </>
      )}
      {error && (
        <p role="alert" className="mt-4 break-all text-sm text-red-600">
          {error}
        </p>
      )}
      {busy && (
        <p role="status" className="muted mt-3 text-xs">
          正在执行本机操作…
        </p>
      )}
    </section>
  );
}
