import { useEffect, useState } from "react";
import { native } from "../platform";
import type { Profile, Profiles } from "../local-system";

const empty: Profile = {
  id: "",
  name: "",
  kind: "remote",
  endpoint: "",
  ca_certificate: null,
};
export default function ProfilesPanel() {
  const [profiles, setProfiles] = useState<Profiles | null>(null);
  const [editing, setEditing] = useState<Profile>(empty);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let disposed = false;
    void native<Profiles>("profiles_get")
      .then((data) => {
        if (!disposed) setProfiles(data);
      })
      .catch((e) => {
        if (!disposed) setError(String(e));
      });
    return () => {
      disposed = true;
    };
  }, []);
  async function change(value: unknown, navigate: boolean) {
    if (!profiles) return;
    setBusy(true);
    setError("");
    try {
      const latest = await native<Profiles>("profiles_change", {
        expectedRevision: profiles.revision,
        change: value,
      });
      setProfiles(latest);
      setEditing(empty);
      if (
        navigate ||
        profiles.selected !== latest.selected ||
        latest.selected === editing.id
      )
        window.location.assign("./index.html");
    } catch (e) {
      setError(String(e));
      // Keep the editing baseline until the user explicitly reloads concurrent changes.
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel panel-body mt-6">
      <h2 className="font-semibold">连接档案</h2>
      <button
        className="btn mt-3"
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          try {
            setProfiles(await native<Profiles>("profiles_get"));
            setEditing(empty);
            setError("");
          } catch (e) {
            setError(String(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        重新读取档案并放弃当前编辑
      </button>
      <p className="muted mt-2 text-xs leading-6">
        档案保存在本机，CLI 和 Desktop 共用。远端档案仅管理目标服务；即使地址是
        localhost，也不会启用本机系统操作。
      </p>
      <ul className="my-5 divide-y divide-console-border">
        {profiles?.profiles.map((p) => (
          <li
            key={p.id}
            className="flex flex-wrap items-center justify-between gap-3 py-3 text-sm"
          >
            <div className="min-w-0">
              <p className="font-medium">
                {p.name}{" "}
                {profiles.selected === p.id && (
                  <span className="text-console-success">· 已选中</span>
                )}
              </p>
              <p className="muted break-all text-xs">
                {p.kind === "local" ? "本机" : "远端"} · {p.endpoint}
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              <button
                className="btn"
                disabled={busy}
                onClick={() =>
                  void change({ action: "select", id: p.id }, true)
                }
              >
                连接 {p.name}
              </button>
              <button
                className="btn"
                disabled={busy}
                onClick={() => setEditing(p)}
              >
                编辑 {p.name}
              </button>
              <button
                className="btn"
                disabled={busy}
                onClick={() =>
                  void change(
                    { action: "remove", id: p.id },
                    p.id === profiles.selected,
                  )
                }
              >
                删除 {p.name}
              </button>
            </div>
          </li>
        ))}
      </ul>
      <form
        className="grid gap-3 sm:grid-cols-2"
        onSubmit={(event) => {
          event.preventDefault();
          void change({ action: "upsert", profile: editing }, false);
        }}
      >
        <label className="form-field">
          档案标识
          <input
            className="field"
            required
            pattern="[a-z0-9][a-z0-9-]{0,47}"
            value={editing.id}
            onChange={(e) => setEditing({ ...editing, id: e.target.value })}
            placeholder="例如 local"
          />
        </label>
        <label className="form-field">
          档案名称
          <input
            className="field"
            required
            value={editing.name}
            maxLength={80}
            onChange={(e) => setEditing({ ...editing, name: e.target.value })}
          />
        </label>
        <label className="form-field">
          档案类型
          <select
            className="field"
            value={editing.kind}
            onChange={(e) =>
              setEditing({
                ...editing,
                kind: e.target.value as Profile["kind"],
              })
            }
          >
            <option value="remote">远端服务</option>
            <option value="local">本机服务 · 允许明确的本机 OS 操作</option>
          </select>
        </label>
        <label className="form-field">
          档案服务地址
          <input
            className="field"
            required
            type="url"
            value={editing.endpoint}
            onChange={(e) =>
              setEditing({ ...editing, endpoint: e.target.value })
            }
          />
        </label>
        <label className="form-field sm:col-span-2">
          可选 CA 证书绝对路径
          <input
            className="field"
            value={editing.ca_certificate ?? ""}
            onChange={(e) =>
              setEditing({ ...editing, ca_certificate: e.target.value || null })
            }
            placeholder="HTTPS 自签名服务可填写 PEM 证书路径"
          />
        </label>
        <div className="flex flex-wrap gap-2 sm:col-span-2">
          <button
            className="btn-primary"
            disabled={busy || !profiles}
            type="submit"
          >
            保存档案
          </button>
          <button
            className="btn"
            type="button"
            disabled={busy}
            onClick={() => setEditing(empty)}
          >
            新建档案
          </button>
        </div>
      </form>
      {error && (
        <p role="alert" className="mt-3 break-all text-sm text-console-danger">
          {error}
        </p>
      )}
    </section>
  );
}
