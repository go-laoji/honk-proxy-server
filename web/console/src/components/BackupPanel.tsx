import { useState } from "react";
import { isDesktop, native, endpoint } from "../platform";
export default function BackupPanel() {
  const [confirmed, setConfirmed] = useState(false);
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState("");
  async function download() {
    setPending(true);
    setMessage("");
    try {
      if (isDesktop) {
        const value = await native<{ saved: boolean; sha256?: string }>(
          "save_database_backup",
        );
        setMessage(
          value.saved ? `备份已保存 · SHA-256 ${value.sha256}` : "已取消保存",
        );
      } else {
        const response = await fetch(
          new URL("api/v1/config/backup", await endpoint()),
          { method: "POST", credentials: "omit" },
        );
        if (!response.ok) {
          const error = (await response.json().catch(() => null)) as {
            message?: string;
          } | null;
          throw new Error(error?.message ?? `备份失败（${response.status}）`);
        }
        const metadata = JSON.parse(
          response.headers.get("X-HPS-Backup") ?? "null",
        ) as { sha256: string; size_bytes: number } | null;
        if (!metadata || metadata.size_bytes > 64 * 1024 * 1024)
          throw new Error("备份元数据无效");
        const blob = await response.blob();
        if (blob.size !== metadata.size_bytes)
          throw new Error("备份下载不完整");
        if (window.crypto.subtle) {
          const digest = [
            ...new Uint8Array(
              await crypto.subtle.digest("SHA-256", await blob.arrayBuffer()),
            ),
          ]
            .map((v) => v.toString(16).padStart(2, "0"))
            .join("");
          if (digest !== metadata.sha256) throw new Error("备份摘要不匹配");
        }
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = `hps-backup-${new Date().toISOString().slice(0, 10)}.sqlite3`;
        link.click();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        setMessage(`备份已下载 · SHA-256 ${metadata.sha256}`);
      }
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setPending(false);
    }
  }
  return (
    <section className="panel panel-body min-w-0" aria-label="数据库备份与恢复">
      <h2 className="font-semibold">数据库备份与恢复</h2>
      <p className="muted mt-3 text-sm leading-7">
        一致性在线快照包含配置历史、上游凭据、节点和任务。请保存到可信的私有位置。普通诊断包不包含完整数据库。
      </p>
      <label className="mt-4 flex items-start gap-3 text-sm">
        <input
          type="checkbox"
          checked={confirmed}
          onChange={(event) => setConfirmed(event.target.checked)}
        />
        我了解备份包含凭据，并会妥善保管。
      </label>
      <button
        className="btn mt-4"
        disabled={!confirmed || pending}
        onClick={() => void download()}
      >
        {pending ? "正在生成一致快照…" : "下载 SQLite 一致备份"}
      </button>
      <p role="status" className="mt-3 break-all text-xs leading-6">
        {message}
      </p>
      <p className="muted mt-4 text-xs leading-6">
        恢复必须先停止目标 Daemon，再在其本机运行离线
        CLI。维护锁阻止运行中替换，原库会保留；完成后显式重启服务。备份不包含外部证书文件、Desktop
        偏好或系统网络配置。
      </p>
      <pre className="code-block mt-3">
        <code>
          hps backup inspect ./hps-backup.sqlite3{"\n"}hps backup restore
          ./hps-backup.sqlite3 --offline --data-dir /private/hps-data --sha256
          &lt;摘要&gt;
        </code>
      </pre>
    </section>
  );
}
