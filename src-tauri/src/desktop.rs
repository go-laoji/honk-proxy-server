//! Desktop-only preferences and OS adapters. Never controls daemon services or network.
use super::*;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_notification::NotificationExt as _;
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Preferences {
    pub close_to_tray: bool,
    pub notifications: bool,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Cursor {
    epoch: String,
    sequence: u64,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct Saved {
    #[serde(default)]
    preferences: Preferences,
    #[serde(default)]
    cursors: BTreeMap<String, Cursor>,
}
pub(super) struct Desktop {
    saved: Mutex<Saved>,
    path: PathBuf,
    pub tray_available: bool,
    error: Mutex<Option<String>>,
}
#[derive(serde::Serialize)]
pub(super) struct View {
    preferences: Preferences,
    login_autostart: bool,
    tray_available: bool,
    delivery_error: Option<String>,
}
impl Desktop {
    pub fn close_to_tray(&self) -> bool {
        self.tray_available && self.saved.lock().is_ok_and(|v| v.preferences.close_to_tray)
    }
    fn view(&self, app: &tauri::AppHandle) -> Result<View, String> {
        Ok(View {
            preferences: self
                .saved
                .lock()
                .map_err(|_| "desktop preferences unavailable")?
                .preferences
                .clone(),
            login_autostart: app.autolaunch().is_enabled().map_err(|e| e.to_string())?,
            tray_available: self.tray_available,
            delivery_error: self
                .error
                .lock()
                .map_err(|_| "notification status unavailable")?
                .clone(),
        })
    }
}
fn persist(path: &Path, saved: &Saved) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("desktop preference directory unavailable")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!("desktop-{}.tmp", std::process::id()));
    let result = (|| {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(saved).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
#[tauri::command]
pub(super) fn desktop_preferences_get(
    app: tauri::AppHandle,
    state: tauri::State<'_, Desktop>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
) -> Result<View, String> {
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    state.view(&app)
}
#[tauri::command]
pub(super) fn desktop_preferences_set(
    app: tauri::AppHandle,
    state: tauri::State<'_, Desktop>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
    preferences: Preferences,
    login_autostart: bool,
) -> Result<View, String> {
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    if preferences.close_to_tray && !state.tray_available {
        return Err("system tray is unavailable; keep window close behavior unchanged".into());
    }
    let mut saved = state
        .saved
        .lock()
        .map_err(|_| "desktop preferences unavailable")?;
    if preferences.notifications && !saved.preferences.notifications {
        app.notification()
            .request_permission()
            .map_err(|e| e.to_string())?;
    }
    let was_enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    if login_autostart != was_enabled {
        if login_autostart {
            app.autolaunch().enable()
        } else {
            app.autolaunch().disable()
        }
        .map_err(|e| e.to_string())?;
    }
    let mut next = saved.clone();
    if preferences.notifications && !saved.preferences.notifications {
        next.cursors.clear();
    }
    next.preferences = preferences;
    if let Err(error) = persist(&state.path, &next) {
        if login_autostart != was_enabled {
            let _ = if was_enabled {
                app.autolaunch().enable()
            } else {
                app.autolaunch().disable()
            };
        }
        return Err(error);
    }
    *saved = next;
    *state
        .error
        .lock()
        .map_err(|_| "notification status unavailable")? = None;
    drop(saved);
    state.view(&app)
}
#[derive(serde::Deserialize)]
struct Notice {
    sequence: u64,
    state: String,
    code: String,
    message: String,
}
#[derive(serde::Deserialize)]
struct Feed {
    epoch: String,
    latest_sequence: u64,
    notices: Vec<Notice>,
    #[serde(default)]
    truncated: bool,
}
// Validate the whole feed before any OS effect. None means establish a fresh baseline.
fn delivery_plan(cursor: Option<&Cursor>, feed: &Feed) -> Result<Option<Vec<usize>>, String> {
    if !hps_domain::valid_connection_epoch(&feed.epoch) || feed.notices.len() > 100 {
        return Err("invalid alert feed".into());
    }
    let Some(cursor) = cursor.filter(|v| v.epoch == feed.epoch) else {
        return Ok(None);
    };
    if feed.latest_sequence < cursor.sequence {
        return Ok(None);
    }
    let mut sequence = cursor.sequence;
    let mut indices = Vec::new();
    for (index, notice) in feed.notices.iter().enumerate() {
        if notice.sequence <= sequence
            || notice.sequence > feed.latest_sequence
            || !["problem", "recovered"].contains(&notice.state.as_str())
            || notice.message.len() > 1024
            || notice.code.len() > 96
        {
            return Err("invalid alert sequence or content".into());
        }
        sequence = notice.sequence;
        indices.push(index);
    }
    Ok(Some(indices))
}
async fn poll(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<Desktop>();
    if !state
        .saved
        .lock()
        .map_err(|_| "desktop preferences unavailable")?
        .preferences
        .notifications
    {
        return Ok(());
    }
    let target = app
        .state::<Connection>()
        .0
        .lock()
        .map_err(|_| "connection unavailable")?
        .clone();
    let key = format!(
        "{}:{}",
        target.profile_id.as_deref().unwrap_or("address"),
        target.endpoint
    );
    let cursor = state
        .saved
        .lock()
        .map_err(|_| "desktop preferences unavailable")?
        .cursors
        .get(&key)
        .cloned();
    let path = format!(
        "api/v1/observability/alerts?limit=100{}",
        cursor
            .as_ref()
            .map(|v| format!("&after={}", v.sequence))
            .unwrap_or_default()
    );
    let feed: Feed = target
        .client(Duration::from_secs(10))
        .map_err(|e| e.to_string())?
        .get_json(&path)
        .await
        .map_err(|e| e.to_string())?;
    let plan = delivery_plan(cursor.as_ref(), &feed)?;
    // Switches and disabling notifications invalidate every in-flight result.
    let connection = app.state::<Connection>();
    let target_guard = connection.0.lock().map_err(|_| "connection unavailable")?;
    if *target_guard != target {
        return Ok(());
    }
    // Keep the target stable through this synchronous delivery batch. Target switches wait here.
    let mut saved = state
        .saved
        .lock()
        .map_err(|_| "desktop preferences unavailable")?;
    if !saved.preferences.notifications {
        return Ok(());
    }
    if plan.is_none() {
        saved.cursors.insert(
            key,
            Cursor {
                epoch: feed.epoch,
                sequence: feed.latest_sequence,
            },
        );
        while saved.cursors.len() > 20 {
            saved.cursors.pop_first();
        }
        persist(&state.path, &saved)?;
        return Ok(());
    }
    for index in plan.unwrap() {
        let notice = &feed.notices[index];
        let sequence = notice.sequence;
        // Reserve before queueing: a crash must not replay OS notifications. Delivery failures remain visible.
        saved.cursors.insert(
            key.clone(),
            Cursor {
                epoch: feed.epoch.clone(),
                sequence,
            },
        );
        persist(&state.path, &saved)?;
        app.notification()
            .builder()
            .title(if notice.state == "recovered" {
                "Honk Proxy · 已恢复"
            } else {
                "Honk Proxy · 服务告警"
            })
            .body(format!("{} · {}", notice.code, notice.message))
            .show()
            .map_err(|_| {
                "OS notification queue rejected delivery; inspect the alert feed".to_owned()
            })?;
    }
    if feed.truncated {
        return Err("Alert retention gap: only retained notices were delivered; inspect the Daemon alert feed".into());
    }
    Ok(())
}
pub(super) fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };
    // Name includes the application identifier, so acceptance bundles own a distinct login entry.
    app.handle().plugin(
        tauri_plugin_autostart::Builder::new()
            .app_name(format!("{}.desktop-login", app.config().identifier))
            .arg("--desktop-login")
            .build(),
    )?;
    let show = MenuItem::with_id(app, "show", "显示 Honk Proxy", true, None::<&str>)?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        "退出 Desktop（Daemon 继续运行）",
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("hps-desktop")
        .tooltip("Honk Proxy Desktop")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    let tray_available = tray.build(app).is_ok();
    let path = app.path().app_config_dir()?.join("desktop.json");
    let saved = std::fs::read(&path)
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .unwrap_or_default();
    app.manage(Desktop {
        saved: Mutex::new(saved),
        path,
        tray_available,
        error: Mutex::new(None),
    });
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let result = poll(&handle).await;
            if let Err(error) = result {
                *handle.state::<Desktop>().error.lock().unwrap() = Some(error);
            }
        }
    });
    Ok(())
}
fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alert_baselines_skip_history_and_only_same_epoch_new_sequences_deliver() {
        let epoch = "12345678-1234-4321-8234-123456789abc";
        let mut feed = Feed {
            epoch: epoch.into(),
            latest_sequence: 5,
            truncated: false,
            notices: vec![Notice {
                sequence: 5,
                state: "problem".into(),
                code: "engine_unready".into(),
                message: "Service unready".into(),
            }],
        };
        assert!(delivery_plan(None, &feed).unwrap().is_none());
        assert!(
            delivery_plan(
                Some(&Cursor {
                    epoch: "other".into(),
                    sequence: 3
                }),
                &feed
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(
            delivery_plan(
                Some(&Cursor {
                    epoch: epoch.into(),
                    sequence: 3
                }),
                &feed
            )
            .unwrap(),
            Some(vec![0])
        );
        assert!(
            delivery_plan(
                Some(&Cursor {
                    epoch: epoch.into(),
                    sequence: 5
                }),
                &feed
            )
            .is_err()
        );
        feed.notices[0].state = "arbitrary".into();
        assert!(
            delivery_plan(
                Some(&Cursor {
                    epoch: epoch.into(),
                    sequence: 3
                }),
                &feed
            )
            .is_err()
        );
    }
    #[test]
    fn defaults_do_not_enable_os_side_effects_and_cursors_roundtrip() {
        let mut saved = Saved::default();
        assert!(!saved.preferences.notifications && !saved.preferences.close_to_tray);
        saved.cursors.insert(
            "test-target".into(),
            Cursor {
                epoch: "epoch".into(),
                sequence: 23,
            },
        );
        let dir = std::env::temp_dir().join(format!("hps-desktop-prefs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        persist(&dir.join("desktop.json"), &saved).unwrap();
        let decoded: Saved =
            serde_json::from_slice(&std::fs::read(dir.join("desktop.json")).unwrap()).unwrap();
        assert_eq!(decoded.cursors["test-target"].sequence, 23);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
