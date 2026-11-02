use hps_client::HpsClient;
use std::{sync::Mutex, time::Duration};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

struct Connection(Mutex<String>);

#[tauri::command]
fn get_endpoint(state: tauri::State<'_, Connection>) -> Result<String, String> {
    state
        .0
        .lock()
        .map(|s| s.clone())
        .map_err(|_| "connection state unavailable".into())
}

#[tauri::command]
fn set_endpoint(
    app: tauri::AppHandle,
    state: tauri::State<'_, Connection>,
    endpoint: String,
) -> Result<(), String> {
    let client = HpsClient::new(&endpoint, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    let endpoint = client.endpoint().as_str().to_owned();
    let mut current = state.0.lock().map_err(|_| "connection state unavailable")?;
    let directory = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let temporary = directory.join("connection.json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec(&endpoint).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temporary, directory.join("connection.json")).map_err(|e| e.to_string())?;
    *current = endpoint;
    Ok(())
}

fn supported_api(path: &str) -> bool {
    matches!(
        path,
        "api/v1/status"
            | "api/v1/version"
            | "api/v1/capabilities"
            | "api/v1/config"
            | "api/v1/linux/preflight"
            | "api/v1/inbounds"
            | "api/v1/config/workspace"
            | "api/v1/config/export"
            | "api/v1/providers"
    ) || supported_provider_get(path)
        || path
            .strip_prefix("api/v1/config/revisions/")
            .is_some_and(|id| id.parse::<i64>().is_ok_and(|id| id > 0))
}

fn valid_resource_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn supported_provider_get(path: &str) -> bool {
    let (base, query) = path
        .split_once('?')
        .map_or((path, None), |(base, query)| (base, Some(query)));
    let parts: Vec<_> = base.split('/').collect();
    match parts.as_slice() {
        ["api", "v1", "provider-tasks", id] => {
            query.is_none() && id.parse::<i64>().is_ok_and(|n| n > 0)
        }
        ["api", "v1", "providers", id] => query.is_none() && valid_resource_id(id),
        ["api", "v1", "providers", id, "leases"] => {
            valid_resource_id(id)
                && query.is_none_or(|q| {
                    !q.is_empty()
                        && q.split('&').all(|pair| {
                            pair.split_once('=').is_some_and(|(k, v)| {
                                matches!(k, "before" | "limit")
                                    && v.parse::<i64>()
                                        .is_ok_and(|n| n > 0 && (k != "limit" || n <= 200))
                            })
                        })
                })
        }
        _ => false,
    }
}

fn supported_action(path: &str) -> bool {
    if matches!(
        path,
        "api/v1/config/validate"
            | "api/v1/config/draft"
            | "api/v1/config/revisions"
            | "api/v1/config/activate"
            | "api/v1/config/rollback"
    ) {
        return true;
    }
    let parts: Vec<_> = path.split('/').collect();
    if matches!(parts.as_slice(),["api","v1","providers",id,"save"|"delete"|"extract"] if valid_resource_id(id))
        || matches!(parts.as_slice(),["api","v1","providers",id,"diagnose","quota"|"whitelist"] if valid_resource_id(id))
    {
        return true;
    }
    matches!(parts.as_slice(),["api","v1","inbounds",id,"start"|"stop"] if !id.is_empty() && id.len()<=64 && id.bytes().all(|c|c.is_ascii_alphanumeric() || c==b'_' || c==b'-'))
}

#[tauri::command]
async fn api_post(
    state: tauri::State<'_, Connection>,
    path: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if !supported_action(&path) {
        return Err("unsupported API operation".into());
    }
    let endpoint = state
        .0
        .lock()
        .map_err(|_| "connection state unavailable")?
        .clone();
    let client = HpsClient::new(&endpoint, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    match body {
        Some(body) => client.post_body(&path, &body).await,
        None => client.post_json(&path).await,
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn save_config_export(
    app: tauri::AppHandle,
    state: tauri::State<'_, Connection>,
    revision: i64,
) -> Result<bool, String> {
    let endpoint = state
        .0
        .lock()
        .map_err(|_| "connection state unavailable")?
        .clone();
    let export = HpsClient::new(&endpoint, Duration::from_secs(10))
        .map_err(|error| error.to_string())?
        .export_config(Some(revision))
        .await
        .map_err(|error| error.to_string())?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(format!("hps-config-r{revision}.toml"))
        .add_filter("TOML", &["toml"])
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = receiver.await.map_err(|_| "save dialog did not complete")? else {
        return Ok(false);
    };
    let path = path
        .into_path()
        .map_err(|_| "selected file path is unavailable")?;
    tokio::task::spawn_blocking(move || std::fs::write(path, export.toml))
        .await
        .map_err(|_| "file save could not finish")?
        .map_err(|error| error.to_string())?;
    Ok(true)
}

#[tauri::command]
async fn api_get(
    state: tauri::State<'_, Connection>,
    path: String,
) -> Result<serde_json::Value, String> {
    if !supported_api(&path) {
        return Err("unsupported API path".into());
    }
    let endpoint = state
        .0
        .lock()
        .map_err(|_| "connection state unavailable")?
        .clone();
    HpsClient::new(&endpoint, Duration::from_secs(10))
        .map_err(|e| e.to_string())?
        .get_json(&path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_web_build_id() -> &'static str {
    hps_web_assets::BUILD_ID
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let saved = app
                .path()
                .app_config_dir()
                .ok()
                .and_then(|p| std::fs::read(p.join("connection.json")).ok())
                .and_then(|bytes| serde_json::from_slice::<String>(&bytes).ok())
                .filter(|endpoint| HpsClient::new(endpoint, Duration::from_secs(10)).is_ok());
            app.manage(Connection(Mutex::new(
                saved.unwrap_or_else(|| "http://127.0.0.1:17900/".into()),
            )));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_endpoint,
            set_endpoint,
            api_get,
            api_post,
            save_config_export,
            get_web_build_id
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_accepts_implemented_api_only() {
        assert!(supported_api("api/v1/status"));
        assert!(supported_api(
            "api/v1/providers/source/leases?before=200&limit=50"
        ));
        assert!(supported_api("api/v1/provider-tasks/1"));
        assert!(!supported_api(
            "api/v1/providers/source/leases?before=1&url=http://external"
        ));
        assert!(!supported_api("api/v1/providers/../config"));
        assert!(supported_action("api/v1/providers/source/extract"));
        assert!(supported_action(
            "api/v1/providers/source/diagnose/whitelist"
        ));
        assert!(!supported_action(
            "api/v1/providers/source/diagnose/purchase"
        ));
        assert!(!supported_api("https://example.com/api/v1/status"));
        assert!(!supported_api("api/v1/../../private"));
        assert!(supported_action("api/v1/inbounds/browser-http/start"));
        assert!(supported_action("api/v1/inbounds/browser-http/stop"));
        assert!(supported_action("api/v1/config/activate"));
        assert!(supported_api("api/v1/config/revisions/12"));
        assert!(!supported_api("api/v1/config/revisions/1/../../private"));
        assert!(!supported_action("api/v1/inbounds/../start"));
        assert!(!supported_action("api/v1/inbounds/browser/start?other=1"));
    }
}
