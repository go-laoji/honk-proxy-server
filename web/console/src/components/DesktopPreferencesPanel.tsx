import { useEffect, useState } from "react";
import { native } from "../platform";
type View = {
  preferences: { close_to_tray: boolean; notifications: boolean };
  login_autostart: boolean;
  tray_available: boolean;
  delivery_error: string | null;
};
export default function DesktopPreferencesPanel() {
  const [value, setValue] = useState<View | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  useEffect(() => {
    let active = true;
    void native<View>("desktop_preferences_get")
      .then((v) => {
        if (active) setValue(v);
      })
      .catch((e) => {
        if (active) setMessage(String(e));
      });
    return () => {
      active = false;
    };
  }, []);
  async function save() {
    if (!value) return;
    setBusy(true);
    setMessage("");
    try {
      setValue(
        await native<View>("desktop_preferences_set", {
          preferences: value.preferences,
          loginAutostart: value.login_autostart,
        }),
      );
      setMessage("Desktop 偏好已保存");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel p-5" aria-label="Desktop 系统体验">
      <h2 className="font-semibold">Desktop 系统体验</h2>
      <p className="muted mt-3 text-xs leading-6">
        托盘可显示窗口或退出 Desktop。退出不会停止独立
        Daemon；关闭到托盘后，通知仍可接收。
      </p>
      {value && (
        <div className="mt-4 space-y-4 text-sm">
          <label className="flex gap-3">
            <input
              type="checkbox"
              disabled={!value.tray_available || busy}
              checked={value.preferences.close_to_tray}
              onChange={(e) =>
                setValue({
                  ...value,
                  preferences: {
                    ...value.preferences,
                    close_to_tray: e.target.checked,
                  },
                })
              }
            />
            关闭窗口时保留在托盘
            {!value.tray_available && "（当前系统托盘不可用）"}
          </label>
          <label className="flex gap-3">
            <input
              type="checkbox"
              disabled={busy}
              checked={value.preferences.notifications}
              onChange={(e) =>
                setValue({
                  ...value,
                  preferences: {
                    ...value.preferences,
                    notifications: e.target.checked,
                  },
                })
              }
            />
            接收当前目标的 OS 告警与恢复通知
          </label>
          <p className="muted text-xs leading-6">
            使用 Daemon
            的去抖告警源；首次连接与重新启用时从最新序号开始，本地按实例 epoch /
            序号去重。仅 Desktop 运行期间投递，系统勿扰或通知权限可能影响展示。
          </p>
          <label className="flex gap-3">
            <input
              type="checkbox"
              disabled={busy}
              checked={value.login_autostart}
              onChange={(e) =>
                setValue({ ...value, login_autostart: e.target.checked })
              }
            />
            当前用户登录时启动 Desktop
          </label>
          <p className="muted text-xs leading-6">
            此项只管理 Desktop 专属登录项。Daemon
            开机启动由下面的本机服务管理单独设置；两者互不启停。
          </p>
          {value.delivery_error && (
            <p role="alert" className="break-all text-xs text-amber-700">
              最近通知投递：{value.delivery_error}
            </p>
          )}
          <button className="btn" disabled={busy} onClick={() => void save()}>
            {busy ? "正在保存…" : "保存 Desktop 偏好"}
          </button>
        </div>
      )}
      <p role="status" className="mt-3 text-xs">
        {message}
      </p>
    </section>
  );
}
