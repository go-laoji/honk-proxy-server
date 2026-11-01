use hps_client::HpsClient;
use std::{sync::Mutex, time::Duration};
use tauri::Manager;

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
    )
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
        assert!(!supported_api("https://example.com/api/v1/status"));
        assert!(!supported_api("api/v1/../../private"));
    }
}
