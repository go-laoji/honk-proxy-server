use super::*;
use axum::extract::{DefaultBodyLimit, Query};
use hps_config::DnsSave;
use hps_dns::{ClearInput, DomainContext, LookupInput, LookupResult};
use serde::Deserialize;
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/dns", get(overview))
        .route("/api/v1/dns/resolve", post(resolve))
        .route("/api/v1/dns/cache/clear", post(clear))
        .route("/api/v1/dns/context", get(context))
        .route("/api/v1/dns/save", post(save))
        .layer(DefaultBodyLimit::max(512 * 1024))
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    after: Option<String>,
    limit: Option<usize>,
}
async fn overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> ApiResult<hps_application::DnsOverview> {
    application(&state)?
        .dns_overview(query.after.as_deref(), query.limit.unwrap_or(50))
        .map(Json)
        .map_err(application_error)
}
async fn resolve(
    State(state): State<Arc<AppState>>,
    Json(input): Json<LookupInput>,
) -> ApiResult<LookupResult> {
    application(&state)?
        .current()
        .dns
        .diagnose(input)
        .await
        .map(Json)
        .map_err(|e| application_error(e.into()))
}
#[derive(serde::Serialize)]
struct ClearResult {
    cleared: usize,
    epoch: u64,
}
async fn clear(
    State(state): State<Arc<AppState>>,
    Json(input): Json<ClearInput>,
) -> ApiResult<ClearResult> {
    let current = application(&state)?.current();
    let cleared = current
        .dns
        .clear(input.domain.as_deref())
        .map_err(|e| application_error(e.into()))?;
    Ok(Json(ClearResult {
        cleared,
        epoch: current
            .dns
            .cache(None, 1)
            .map_err(|e| application_error(e.into()))?
            .epoch,
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextQuery {
    ip: std::net::IpAddr,
}
async fn context(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ContextQuery>,
) -> ApiResult<DomainContext> {
    Ok(Json(
        application(&state)?.current().dns.domain_context(query.ip),
    ))
}
async fn save(
    State(state): State<Arc<AppState>>,
    Json(input): Json<DnsSave>,
) -> ApiResult<hps_domain::ConfigActivationResult> {
    application(&state)?
        .edit_dns(input)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dns_http_client_exposes_ttl_semantics_strict_inputs_and_revision_cas() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        let service = Supervisor::boot(
            hps_storage::Store::open(None, config).unwrap(),
            hps_application::StartupOverrides::default(),
        )
        .await
        .unwrap();
        service.confirm_startup().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(AppState::with_application(service.clone()));
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::router(state)).await.unwrap();
        });
        let endpoint = format!("http://{address}");
        let client =
            hps_client::HpsClient::new(&endpoint, std::time::Duration::from_secs(3)).unwrap();
        let view = client.dns(None, 50).await.unwrap();
        assert_eq!(view.running_revision, 1);
        assert!(!view.linux_capture_active);
        let result = client
            .resolve_dns(&LookupInput {
                name: "localhost".into(),
                query_type: "A".into(),
                purpose: hps_net::ResolvePurpose::Business,
            })
            .await
            .unwrap();
        assert_eq!(result.remaining_ttl, None);
        assert_eq!(client.dns(None, 50).await.unwrap().cache.total, 0);
        assert!(matches!(
            client
                .resolve_dns(&LookupInput {
                    name: "localhost".into(),
                    query_type: "AXFR".into(),
                    purpose: hps_net::ResolvePurpose::Business
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        let mut policy = view.configuration;
        policy.timeout_ms = 3000;
        assert_eq!(
            client
                .save_dns(&DnsSave {
                    configuration: policy.clone(),
                    expected_revision: 1
                })
                .await
                .unwrap()
                .running_revision,
            2
        );
        assert!(matches!(
            client
                .save_dns(&DnsSave {
                    configuration: policy.clone(),
                    expected_revision: 1
                })
                .await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        policy.listen = Some("127.0.0.1:1053".parse().unwrap());
        assert!(matches!(
            client
                .save_dns(&DnsSave {
                    configuration: policy,
                    expected_revision: 2
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        assert_eq!(client.dns(None, 50).await.unwrap().running_revision, 2);
        assert!(
            client
                .dns_context("192.0.2.100".parse().unwrap())
                .await
                .unwrap()
                .domains
                .is_empty()
        );
        assert_eq!(
            client
                .clear_dns(&ClearInput { domain: None })
                .await
                .unwrap()["epoch"],
            1
        );
        let http = reqwest::Client::new();
        assert_eq!(
            http.get(format!("{endpoint}/api/v1/dns?limit=201"))
                .send()
                .await
                .unwrap()
                .status(),
            422
        );
        assert_eq!(
            http.get(format!("{endpoint}/api/v1/dns?unexpected=true"))
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
        service.shutdown().await.unwrap();
        server.abort();
    }
}
