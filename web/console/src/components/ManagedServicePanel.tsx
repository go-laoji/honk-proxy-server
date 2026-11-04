import { useEffect, useState } from "react";
import { ArrowRight, Play, Square } from "lucide-react";
import { native } from "../platform";
import { Badge } from "./ui";

type Status = { endpoint: string | null; selected: boolean };

export default function ManagedServicePanel() {
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState<"start" | "stop" | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let mounted = true;
    void native<Status>("managed_service_status")
      .then((result) => {
        if (mounted) setStatus(result);
      })
      .catch((cause: unknown) => {
        if (mounted)
          setError(cause instanceof Error ? cause.message : String(cause));
      });
    return () => {
      mounted = false;
    };
  }, []);

  async function control(action: "start" | "stop") {
    setBusy(action);
    setError("");
    try {
      if (action === "start") {
        await native("managed_service_start");
        window.location.assign("./overview.html");
      } else {
        await native("managed_service_stop");
        window.location.replace("./index.html");
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      setBusy(null);
    }
  }

  const running = Boolean(status?.endpoint);
  return (
    <section
      className="panel panel-body space-y-3"
      aria-labelledby="managed-service-heading"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 id="managed-service-heading" className="font-semibold">
          Desktop 本地服务
        </h2>
        <Badge active={running}>
          {running
            ? "运行中"
            : status
              ? "已停止"
              : error
                ? "状态不可用"
                : "读取状态中"}
        </Badge>
      </div>
      <p className="muted text-sm leading-6">
        可在 Desktop 内启动代理服务。配置和数据会保留，退出 Desktop
        时服务会停止。
      </p>
      {status?.endpoint && (
        <p className="muted text-xs">
          {status.selected ? "当前连接 · " : "管理地址 · "}
          <code className="break-all">{status.endpoint}</code>
        </p>
      )}
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          className="btn-primary"
          disabled={busy !== null || (!status && !error)}
          onClick={() => void control("start")}
        >
          {running ? (
            <ArrowRight size={16} aria-hidden="true" />
          ) : (
            <Play size={16} aria-hidden="true" />
          )}
          {busy === "start"
            ? running
              ? "正在打开…"
              : "正在启动…"
            : running
              ? "打开本地服务"
              : "启动本地服务"}
        </button>
        {running && (
          <button
            type="button"
            className="btn"
            disabled={busy !== null}
            onClick={() => void control("stop")}
          >
            <Square size={15} aria-hidden="true" />
            {busy === "stop" ? "正在停止…" : "停止本地服务"}
          </button>
        )}
      </div>
      {error && (
        <p role="alert" className="break-words text-sm text-console-danger">
          {error}
        </p>
      )}
    </section>
  );
}
