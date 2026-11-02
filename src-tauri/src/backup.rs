use super::*;
#[tauri::command]
pub(super) async fn save_database_backup(
    app: tauri::AppHandle,
    state: tauri::State<'_, Connection>,
    subscriptions: tauri::State<'_, events::NativeEvents>,
    window: tauri::WebviewWindow,
    context: events::SubscriptionContext,
) -> Result<serde_json::Value, String> {
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    let target = state
        .0
        .lock()
        .map_err(|_| "connection state unavailable")?
        .clone();
    let (metadata, bytes) = target
        .client(Duration::from_secs(45))
        .map_err(|e| e.to_string())?
        .database_backup()
        .await
        .map_err(|e| e.to_string())?;
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name("hps-backup.sqlite3")
        .add_filter("SQLite backup (contains credentials)", &["sqlite3"])
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = receiver.await.map_err(|_| "save dialog unavailable")? else {
        return Ok(serde_json::json!({"saved":false}));
    };
    subscriptions
        .validate_context(window.label(), context)
        .map_err(|e| e.message)?;
    let path = path.into_path().map_err(|_| "selected path unavailable")?;
    tokio::task::spawn_blocking(move || hps_storage::backup::save_new(&path, &bytes))
        .await
        .map_err(|_| "backup file worker unavailable")?
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"saved":true,"sha256":metadata.sha256}))
}
