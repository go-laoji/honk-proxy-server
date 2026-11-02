import { useState, useEffect } from "react";
import { Cable, ArrowRight } from "lucide-react";
import type { ServiceData } from "../use-service";
import { Badge } from "../components/ui";
import { endpoint, connect, isDesktop } from "../platform";
import ProfilesPanel from "../components/ProfilesPanel";
import LocalSystemPanel from "../components/LocalSystemPanel";

export default function ConnectPage({
  data,
  refreshService,
}: {
  data: ServiceData | null;
  refreshService: () => Promise<void>;
}) {
  const [address, setAddress] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    void endpoint()
      .then(setAddress)
      .catch((error) => setError(String(error)));
  }, []);
  return (
    <div className="mx-auto max-w-2xl">
      <section className="panel overflow-hidden">
        <div className="border-b border-slate-100 p-7 dark:border-slate-800">
          <div className="mb-5 inline-flex rounded-2xl bg-blue-50 p-4 text-blue-600 dark:bg-blue-950">
            <Cable size={30} />
          </div>
          <h2 className="text-xl font-semibold">连接你的代理服务</h2>
          <p className="muted mt-2 text-sm leading-7">
            选择本机或远端 Daemon，管理所有可用功能。无需登录或账号。
          </p>
        </div>
        <form
          className="space-y-5 p-7"
          onSubmit={async (event) => {
            event.preventDefault();
            setSaving(true);
            setError("");
            try {
              await connect(address);
            } catch (error) {
              setError(error instanceof Error ? error.message : String(error));
              setSaving(false);
            }
          }}
        >
          <div>
            <label
              className="mb-2 block text-sm font-medium"
              htmlFor="endpoint"
            >
              服务地址
            </label>
            <input
              className="field"
              id="endpoint"
              type="url"
              value={address}
              required
              onChange={(event) => setAddress(event.target.value)}
              placeholder="http://127.0.0.1:17900"
            />
            <p className="muted mt-2 text-xs">
              使用服务的根地址，包含 HTTP 或 HTTPS 协议。
            </p>
          </div>
          {error && (
            <p role="alert" className="text-sm text-red-600">
              {error}
            </p>
          )}
          <button className="btn-primary" type="submit" disabled={saving}>
            {saving ? "正在连接…" : "连接服务"}
            <ArrowRight size={16} />
          </button>
        </form>
        <div className="flex items-center justify-between border-t border-slate-100 px-7 py-4 dark:border-slate-800">
          <span className="muted text-xs">
            {isDesktop ? "Desktop 原生连接" : "浏览器直接连接"}
          </span>
          <Badge active={Boolean(data)}>
            {data ? "当前服务可达" : "等待连接"}
          </Badge>
        </div>
      </section>
      {isDesktop && (
        <>
          <ProfilesPanel />
          <LocalSystemPanel data={data} refreshService={refreshService} />
        </>
      )}
    </div>
  );
}
