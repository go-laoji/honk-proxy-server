use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use hps_application::{ApplicationError, Supervisor};
use hps_config::DaemonConfig;
use hps_domain::{ApiError, Capabilities, ServiceStatus, VersionInfo};
use hps_domain::{InboundStatus, ProxyStatus};
use hps_engine::runtime::{Engine, EngineError};
use hps_storage::StorageError;
use std::{
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod configuration;
mod dns;
mod groups;
mod health;
mod providers;
mod rules;
#[cfg(feature = "web-ui")]
mod web;

pub struct AppState {
    pub config: DaemonConfig,
    pub engine: Option<Arc<Engine>>,
    pub application: Option<Arc<Supervisor>>,
    instance_id: String,
    started_at: Instant,
    started_at_unix: u64,
}

impl AppState {
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            config,
            engine: None,
            application: None,
            instance_id: uuid::Uuid::new_v4().to_string(),
            started_at: Instant::now(),
            started_at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }

    pub fn with_engine(config: DaemonConfig, engine: Arc<Engine>) -> Self {
        let mut state = Self::new(config);
        state.engine = Some(engine);
        state
    }
    pub fn with_application(application: Arc<Supervisor>) -> Self {
        let snapshot = application.current();
        let mut state = Self::with_engine(snapshot.config.clone(), snapshot.engine.clone());
        state.application = Some(application);
        state
    }
    pub fn effective_config(&self) -> DaemonConfig {
        self.application.as_ref().map_or_else(
            || self.config.clone(),
            |application| application.current().config.clone(),
        )
    }
    pub fn runtime_engine(&self) -> Option<Arc<Engine>> {
        self.application
            .as_ref()
            .map(|application| application.current().engine.clone())
            .or_else(|| self.engine.clone())
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
        let snapshot = self
            .application
            .as_ref()
            .map(|application| application.current());
        let config = snapshot
            .as_ref()
            .map_or(&self.config, |snapshot| &snapshot.config);
        let engine = snapshot
            .as_ref()
            .map(|snapshot| &snapshot.engine)
            .or(self.engine.as_ref());
        ServiceStatus {
            instance_id: self.instance_id.clone(),
            instance_name: config.instance_name.clone(),
            management_ready: true,
            proxy_ready: engine.is_some_and(|engine| engine.ready()),
            engine_state: engine
                .map_or("not_initialized", |engine| {
                    if engine.ready() {
                        "running"
                    } else {
                        let status = engine.status();
                        if status.shutting_down {
                            "stopping"
                        } else if status.inbounds.iter().any(|entry| entry.running) {
                            "no_eligible_upstream"
                        } else {
                            "no_running_inbound"
                        }
                    }
                })
                .into(),
            started_at_unix: self.started_at_unix,
            uptime_seconds: self.started_at.elapsed().as_secs(),
            platform: std::env::consts::OS.into(),
            version: self.version(),
        }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    let router = Router::new()
        .route(
            "/healthz",
            get(|| async { Json(serde_json::json!({"alive": true})) }),
        )
        .route("/readyz", get(ready))
        .route("/api/v1/status", get(status))
        .route("/api/v1/version", get(version))
        .route("/api/v1/capabilities", get(capabilities))
        .route("/api/v1/linux/preflight", get(linux_preflight))
        .route("/api/v1/inbounds", get(inbounds))
        .route("/api/v1/inbounds/{id}/start", post(start_inbound))
        .route("/api/v1/inbounds/{id}/stop", post(stop_inbound))
        .route("/api/v1/config", get(config));
    let router = router
        .merge(configuration::router())
        .merge(providers::router())
        .merge(health::router())
        .merge(dns::router())
        .merge(groups::router())
        .merge(rules::router())
        .layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024));
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
async fn capabilities(State(state): State<Arc<AppState>>) -> Json<Capabilities> {
    Json(if state.runtime_engine().is_some() {
        Capabilities::explicit_proxy()
    } else {
        Capabilities::foundation()
    })
}

async fn ready(State(state): State<Arc<AppState>>) -> (StatusCode, Json<serde_json::Value>) {
    let ready = state.runtime_engine().is_some_and(|engine| engine.ready());
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(
            serde_json::json!({"ready":ready,"reason":if ready {if state.effective_config().health.enabled {"explicit ingress running with fresh healthy upstream"} else {"explicit ingress running; health checking explicitly disabled"}} else {"no ready inbound/upstream or runtime stopping"}}),
        ),
    )
}
async fn inbounds(State(state): State<Arc<AppState>>) -> Json<ProxyStatus> {
    Json(state.runtime_engine().as_ref().map_or_else(
        || ProxyStatus {
            inbounds: vec![],
            active_connections: 0,
            eligible_upstreams: 0,
            limits: state.effective_config().proxy.limits,
            shutting_down: false,
        },
        |engine| engine.status(),
    ))
}
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;
fn engine_error(error: EngineError) -> (StatusCode, Json<ApiError>) {
    let (status, code) = match error {
        EngineError::NotFound => (StatusCode::NOT_FOUND, "inbound_not_found"),
        EngineError::Bind => (StatusCode::CONFLICT, "listen_failed"),
        EngineError::ShuttingDown => (StatusCode::SERVICE_UNAVAILABLE, "runtime_stopping"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "runtime_unavailable"),
    };
    (
        status,
        Json(ApiError {
            code: code.into(),
            message: error.to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            retryable: false,
        }),
    )
}
async fn start_inbound(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<InboundStatus> {
    if let Some(application) = &state.application {
        return application
            .start_inbound(&id)
            .await
            .map(Json)
            .map_err(application_error);
    }
    let engine = state
        .engine
        .as_ref()
        .ok_or_else(|| engine_error(EngineError::ShuttingDown))?;
    engine.start(&id).await.map(Json).map_err(engine_error)
}
async fn stop_inbound(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<InboundStatus> {
    if let Some(application) = &state.application {
        return application
            .stop_inbound(&id)
            .await
            .map(Json)
            .map_err(application_error);
    }
    let engine = state
        .engine
        .as_ref()
        .ok_or_else(|| engine_error(EngineError::ShuttingDown))?;
    engine.stop(&id).await.map(Json).map_err(engine_error)
}
async fn config(State(state): State<Arc<AppState>>) -> Json<DaemonConfig> {
    Json(state.effective_config().redacted())
}

fn application_error(error: ApplicationError) -> (StatusCode, Json<ApiError>) {
    let (status, code) = match &error {
        ApplicationError::Storage(StorageError::Conflict) => {
            (StatusCode::CONFLICT, "configuration_conflict")
        }
        ApplicationError::Storage(StorageError::NotFound) => {
            (StatusCode::NOT_FOUND, "revision_not_found")
        }
        ApplicationError::Invalid
        | ApplicationError::Storage(StorageError::Invalid)
        | ApplicationError::Engine(
            EngineError::Tls | EngineError::Configuration | EngineError::Routing,
        ) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_configuration"),
        ApplicationError::Engine(EngineError::Bind) => (StatusCode::CONFLICT, "listen_failed"),
        ApplicationError::Engine(EngineError::NotFound) => {
            (StatusCode::NOT_FOUND, "inbound_not_found")
        }
        ApplicationError::ProviderUnavailable => (StatusCode::NOT_FOUND, "provider_unavailable"),
        ApplicationError::GroupUnavailable => (StatusCode::NOT_FOUND, "group_not_found"),
        ApplicationError::RuleUnavailable => (StatusCode::NOT_FOUND, "rule_not_found"),
        ApplicationError::RulesBase => (StatusCode::CONFLICT, "rules_candidate_base_changed"),
        ApplicationError::RouteNodeUnavailable => {
            (StatusCode::UNPROCESSABLE_ENTITY, "routing_node_unavailable")
        }
        ApplicationError::Provider(error) => (StatusCode::BAD_GATEWAY, error.code),
        ApplicationError::Capture(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "dns_capture_preparation_failed",
        ),
        ApplicationError::Dns(error) => {
            use hps_dns::DnsError;
            match error {
                DnsError::InvalidQuery | DnsError::Configuration | DnsError::Loop => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "dns_invalid_input")
                }
                DnsError::Disabled => (StatusCode::CONFLICT, "dns_disabled"),
                DnsError::Capacity => (StatusCode::TOO_MANY_REQUESTS, "dns_capacity"),
                DnsError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "dns_timeout"),
                _ => (StatusCode::BAD_GATEWAY, "dns_resolution_failed"),
            }
        }
        ApplicationError::Storage(StorageError::ProviderRateLimited) => {
            (StatusCode::TOO_MANY_REQUESTS, "provider_interval_limit")
        }
        ApplicationError::Storage(StorageError::ProviderPaused) => {
            (StatusCode::CONFLICT, "provider_paused")
        }
        ApplicationError::Storage(StorageError::ProviderBudgetExceeded) => {
            (StatusCode::TOO_MANY_REQUESTS, "provider_budget_exhausted")
        }
        ApplicationError::Storage(StorageError::HealthCapacity) => {
            (StatusCode::TOO_MANY_REQUESTS, "health_capacity_or_budget")
        }
        ApplicationError::ExtractionCapacity => {
            (StatusCode::SERVICE_UNAVAILABLE, "supplier_fetch_capacity")
        }
        ApplicationError::ClockRollback => (StatusCode::CONFLICT, "clock_rollback"),
        ApplicationError::Storage(StorageError::PoolFull) => {
            (StatusCode::CONFLICT, "provider_pool_budget")
        }
        ApplicationError::GenerationBudget => (StatusCode::CONFLICT, "draining_generation_limit"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "configuration_unavailable"),
    };
    (
        status,
        Json(ApiError {
            code: code.into(),
            message: error.to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            retryable: false,
        }),
    )
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

    #[tokio::test]
    async fn inbound_controls_and_readiness_follow_runtime_without_credentials() {
        let config=DaemonConfig::from_toml("[[proxy.inbounds]]\nid='browser'\nprotocol='http'\nlisten='127.0.0.1:0'\nenabled=false\n[[proxy.upstreams]]\nid='bootstrap'\nprotocol='http'\nhost='127.0.0.1'\nport=18080\n[proxy.upstreams.credentials]\nusername='supplier-private'\npassword='credential-private'\n").unwrap();
        let engine = Engine::new(config.proxy.clone()).unwrap();
        engine.start_enabled().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let runtime = engine.clone();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router(Arc::new(AppState::with_engine(config, runtime))),
            )
            .await
            .unwrap();
        });
        let base = format!("http://{address}");
        let client = HpsClient::new(&base, Duration::from_secs(2)).unwrap();
        assert!(!client.status().await.unwrap().proxy_ready);
        assert!(!client.inbounds().await.unwrap().inbounds[0].running);
        let redacted = serde_json::to_string(&client.config().await.unwrap()).unwrap();
        assert!(!redacted.contains("credential-private"));
        assert!(!redacted.contains("supplier-private"));
        assert!(client.start_inbound("browser").await.unwrap().running);
        assert!(client.status().await.unwrap().proxy_ready);
        let caps = client.capabilities().await.unwrap();
        assert!(
            caps.http_proxy.available && caps.https_proxy.available && caps.socks5_proxy.available
        );
        assert!(!caps.requires_authentication && !caps.linux_ebpf.ready);
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        assert_eq!(
            http.get(format!("{base}/readyz"))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert!(!client.stop_inbound("browser").await.unwrap().running);
        assert_eq!(
            http.get(format!("{base}/readyz"))
                .send()
                .await
                .unwrap()
                .status(),
            503
        );
        assert!(matches!(
            client.start_inbound("missing").await,
            Err(hps_client::ClientError::Api { status: 404, .. })
        ));
        engine.shutdown().await;
        task.abort();
    }
}
