use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use hps_config::DaemonConfig;
use hps_domain::{ApiError, Capabilities, ServiceStatus, VersionInfo};
use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "web-ui")]
mod web;

pub struct AppState {
    pub config: DaemonConfig,
    instance_id: String,
    started_at: Instant,
    started_at_unix: u64,
}

impl AppState {
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            config,
            instance_id: uuid::Uuid::new_v4().to_string(),
            started_at: Instant::now(),
            started_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    pub fn version(&self) -> VersionInfo {
        VersionInfo {
            service: "honk-proxy-server".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            api_version: hps_domain::API_VERSION.into(),
            web_build_id: {
                #[cfg(feature = "web-ui")]
                {
                    Some(hps_web_assets::BUILD_ID.into())
                }
                #[cfg(not(feature = "web-ui"))]
                {
                    None
                }
            },
        }
    }

    pub fn status(&self) -> ServiceStatus {
        ServiceStatus {
            instance_id: self.instance_id.clone(),
            instance_name: self.config.instance_name.clone(),
            management_ready: true,
            proxy_ready: false,
            engine_state: "not_initialized".into(),
            started_at_unix: self.started_at_unix,
            uptime_seconds: self.started_at.elapsed().as_secs(),
            platform: std::env::consts::OS.into(),
            version: self.version(),
        }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let router = Router::new()
        .route("/healthz", get(|| async { Json(serde_json::json!({"alive": true})) }))
        .route("/readyz", get(|| async {
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"ready": false, "reason": "proxy runtime is not initialized"})))
        }))
        .route("/api/v1/status", get(status))
        .route("/api/v1/version", get(version))
        .route("/api/v1/capabilities", get(capabilities))
        .route("/api/v1/linux/preflight", get(linux_preflight))
        .route("/api/v1/config", get(config));
    #[cfg(feature = "web-ui")]
    let router = router.fallback(web::serve);
    #[cfg(not(feature = "web-ui"))]
    let router = router.fallback(not_found);
    router.with_state(state)
}

async fn status(State(state): State<Arc<AppState>>) -> Json<ServiceStatus> {
    Json(state.status())
}
async fn version(State(state): State<Arc<AppState>>) -> Json<VersionInfo> {
    Json(state.version())
}
async fn capabilities() -> Json<Capabilities> {
    Json(Capabilities::foundation())
}
async fn config(State(state): State<Arc<AppState>>) -> Json<DaemonConfig> {
    Json(state.config.clone())
}

async fn linux_preflight() -> Result<Json<hps_domain::LinuxPreflight>, (StatusCode, Json<ApiError>)>
{
    tokio::task::spawn_blocking(hps_platform::preflight)
        .await
        .map(Json)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    code: "preflight_failed".into(),
                    message: "Environment diagnostic could not finish".into(),
                    request_id: uuid::Uuid::new_v4().to_string(),
                    retryable: true,
                }),
            )
        })
}

pub async fn not_found() -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            code: "not_found".into(),
            message: "This resource is not available".into(),
            request_id: uuid::Uuid::new_v4().to_string(),
            retryable: false,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_client::HpsClient;
    use std::time::Duration;

    #[tokio::test]
    async fn real_http_client_needs_no_auth_and_does_not_claim_proxy_ready() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router(Arc::new(AppState::new(DaemonConfig::default()))),
            )
            .await
            .unwrap();
        });
        let client = HpsClient::new(&format!("http://{address}"), Duration::from_secs(2)).unwrap();
        let status = client.status().await.unwrap();
        assert!(status.management_ready);
        assert!(!status.proxy_ready);
        let caps = client.capabilities().await.unwrap();
        assert!(!caps.requires_authentication);
        assert!(!caps.linux_ebpf.attached && !caps.http_proxy.available);
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        assert_eq!(
            http.get(format!("http://{address}/readyz"))
                .send()
                .await
                .unwrap()
                .status(),
            503
        );
        assert_eq!(
            http.get(format!("http://{address}/login.html"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
        assert!(client.config().await.is_ok());
        let preflight = client.linux_preflight().await.unwrap();
        assert_eq!(preflight.platform, std::env::consts::OS);
        assert_eq!(preflight.scope, "read_only_prerequisites");
        assert!(!preflight.datapath_ready);
        task.abort();
    }
}
