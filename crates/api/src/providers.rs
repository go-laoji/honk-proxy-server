use super::*;
use axum::extract::{DefaultBodyLimit, Query};
use hps_config::{ProviderConfig, ProviderSave};
use hps_domain::{
    ConfigActivationResult, ExpectedRevision, ExtractionInput, ExtractionTask, LeaseInfo,
    ProviderDetail, ProviderView,
};
use serde::Deserialize;
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/providers", get(list))
        .route("/api/v1/providers/{id}", get(detail))
        .route("/api/v1/providers/{id}/save", post(save))
        .route("/api/v1/providers/{id}/delete", post(remove))
        .route("/api/v1/providers/{id}/extract", post(extract))
        .route("/api/v1/providers/{id}/leases", get(leases))
        .route("/api/v1/providers/{id}/diagnose/{kind}", post(diagnose))
        .route("/api/v1/provider-tasks/{id}", get(task))
        .layer(DefaultBodyLimit::max(1024 * 1024))
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn list(State(state): State<Arc<AppState>>) -> ApiResult<Vec<ProviderView<ProviderConfig>>> {
    application(&state)?
        .providers()
        .await
        .map(Json)
        .map_err(application_error)
}
async fn detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<ProviderDetail<ProviderConfig>> {
    application(&state)?
        .provider_detail(&id)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn save(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ProviderSave>,
) -> ApiResult<ConfigActivationResult> {
    application(&state)?
        .edit_provider(id, Some(input.configuration), input.expected_revision)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn remove(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ExpectedRevision>,
) -> ApiResult<ConfigActivationResult> {
    application(&state)?
        .edit_provider(id, None, input.expected_revision)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn extract(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ExtractionInput>,
) -> Result<(StatusCode, Json<ExtractionTask>), (StatusCode, Json<ApiError>)> {
    application(&state)?
        .extract(id, input)
        .await
        .map(|task| (StatusCode::ACCEPTED, Json(task)))
        .map_err(application_error)
}
#[derive(Deserialize)]
struct LeaseQuery {
    before: Option<i64>,
    limit: Option<u32>,
}
async fn leases(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<LeaseQuery>,
) -> ApiResult<Vec<LeaseInfo>> {
    application(&state)?
        .provider_leases(Some(id), query.before, query.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(application_error)
}
async fn task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> ApiResult<ExtractionTask> {
    application(&state)?
        .extraction_task(id)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn diagnose(
    State(state): State<Arc<AppState>>,
    Path((id, kind)): Path<(String, String)>,
) -> ApiResult<serde_json::Value> {
    application(&state)?
        .provider_diagnostic(&id, &kind)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_client::HpsClient;
    use std::time::Duration;
    #[tokio::test]
    async fn provider_http_crud_cas_task_dedup_and_redacted_lease_details_need_no_account() {
        let vendor = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let vendor_addr = vendor.local_addr().unwrap();
        let vendor_task = tokio::spawn(async move {
            let app = Router::new().route("/extract", get(|| async { "192.0.2.1:8080\n" }));
            axum::serve(vendor, app).await.unwrap();
        });
        let store = hps_storage::Store::open(None, DaemonConfig::default()).unwrap();
        let service = Supervisor::boot(store, hps_application::StartupOverrides::default())
            .await
            .unwrap();
        service.confirm_startup().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(AppState::with_application(service.clone()));
        let server = tokio::spawn(async move {
            axum::serve(listener, super::super::router(state))
                .await
                .unwrap();
        });
        let client = HpsClient::new(&format!("http://{address}"), Duration::from_secs(2)).unwrap();
        assert!(client.providers().await.unwrap().is_empty());
        let config = ProviderConfig {
            id: "vendor".into(),
            enabled: true,
            kind: hps_config::ProviderKind::Text,
            endpoint: format!("http://{vendor_addr}/extract"),
            ttl_secs: Some(120),
            upstream_credentials: Some(hps_config::UpstreamCredentials {
                username: "test".into(),
                password: "private-password".into(),
            }),
            ..Default::default()
        };
        let activated = client.save_provider(config.clone(), 1).await.unwrap();
        assert_eq!(activated.state, "applied");
        assert!(matches!(
            client.save_provider(config, 1).await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        let task = client
            .extract_provider("vendor", "request-1".into())
            .await
            .unwrap();
        let duplicate = client
            .extract_provider("vendor", "request-1".into())
            .await
            .unwrap();
        assert_eq!(task.id, duplicate.id);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let task = client.provider_task(task.id).await.unwrap();
                if task.state == "succeeded" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let detail = client.provider("vendor").await.unwrap();
        assert_eq!(detail.leases.len(), 1);
        assert_eq!(detail.tasks.len(), 1);
        assert_eq!(detail.provider.active_leases, 1);
        assert!(
            !serde_json::to_string(&detail)
                .unwrap()
                .contains("private-password")
        );
        let diagnostic = client.provider_diagnostic("vendor", "quota").await.unwrap();
        assert_eq!(diagnostic["supported"], false);
        client
            .delete_provider("vendor", activated.desired_revision)
            .await
            .unwrap();
        assert!(client.providers().await.unwrap().is_empty());
        assert!(
            client
                .extract_provider("vendor", "request-2".into())
                .await
                .is_err()
        );
        assert!(client.provider("../config").await.is_err());
        service.shutdown().await.unwrap();
        server.abort();
        vendor_task.abort();
    }
}
