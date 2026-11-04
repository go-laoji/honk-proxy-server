import { useState, useEffect } from "react";
import { Cable, ArrowRight } from "lucide-react";
import type { ServiceData } from "../use-service";
import { Badge } from "../components/ui";
import { endpoint, connect, isDesktop } from "../platform";
import ProfilesPanel from "../components/ProfilesPanel";
import LocalSystemPanel from "../components/LocalSystemPanel";
import ManagedServicePanel from "../components/ManagedServicePanel";

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
    <div className="mx-auto max-w-2xl space-y-3">
      {isDesktop && <ManagedServicePanel />}
      <section className="panel overflow-hidden">
        <div className="panel-header flex items-center gap-3">
          <div className="page-heading-mark">
            <Cable size={18} />
          </div>
          <div>
            <h2>连接你的代理服务</h2>
            <p className="muted mt-0.5 text-xs">
              输入已运行的本机或远端服务地址。
            </p>
          </div>
        </div>
        <form
          className="panel-body space-y-3"
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
          <div className="form-field">
            <label htmlFor="endpoint">服务地址</label>
            <input
              className="field"
              id="endpoint"
              type="url"
              value={address}
              required
              onChange={(event) => setAddress(event.target.value)}
              placeholder="http://127.0.0.1:17900"
            />
            <p className="muted font-normal">
              使用服务的根地址，包含 HTTP 或 HTTPS 协议。
            </p>
          </div>
          {error && (
            <p role="alert" className="text-sm text-console-danger">
              {error}
            </p>
          )}
          <button className="btn-primary" type="submit" disabled={saving}>
            {saving ? "正在连接…" : "连接服务"}
            <ArrowRight size={16} />
          </button>
        </form>
        <div className="panel-body panel-toolbar border-t border-console-border">
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
