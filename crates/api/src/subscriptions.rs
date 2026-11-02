use super::*;
use hps_config::{SubscriptionRefresh, SubscriptionSave, SubscriptionsOverview};
use hps_domain::{ConfigActivationResult, ExpectedRevision};
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/subscriptions", get(list))
        .route("/api/v1/subscriptions/{id}/save", post(save))
        .route("/api/v1/subscriptions/{id}/delete", post(delete))
        .route("/api/v1/subscriptions/{id}/refresh", post(refresh))
        .layer(axum::extract::DefaultBodyLimit::max(1100 * 1024))
}
fn app(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn list(State(state): State<Arc<AppState>>) -> ApiResult<SubscriptionsOverview> {
    Ok(Json(app(&state)?.subscriptions()))
}
async fn save(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<SubscriptionSave>,
) -> ApiResult<ConfigActivationResult> {
    app(&state)?
        .edit_subscription(id, Some(input.configuration), input.expected_revision)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<ExpectedRevision>,
) -> ApiResult<ConfigActivationResult> {
    app(&state)?
        .edit_subscription(id, None, input.expected_revision)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn refresh(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(input): Json<SubscriptionRefresh>,
) -> ApiResult<ConfigActivationResult> {
    app(&state)?
        .refresh_subscription(id, input)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn typed_client_subscription_crud_refresh_conflict_and_node_controls_are_real_http() {
        use hps_config::{SubscriptionConfig, SubscriptionFormat};
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        let store = hps_storage::Store::open(None, config).unwrap();
        let service = Supervisor::boot(store, Default::default()).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = super::super::router(Arc::new(AppState::with_application(service.clone())));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = hps_client::HpsClient::new(
            &format!("http://{address}"),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        let initial = client.subscriptions().await.unwrap();
        assert!(initial.subscriptions.is_empty());
        let saved = client
            .save_subscription(&SubscriptionSave {
                configuration: SubscriptionConfig {
                    id: "fixture".into(),
                    url: "https://example.test/sub?secret=hidden-token".into(),
                    format: SubscriptionFormat::Uri,
                    ..Default::default()
                },
                expected_revision: initial.running_revision,
            })
            .await
            .unwrap();
        let refreshed = client
            .refresh_subscription(
                "fixture",
                &SubscriptionRefresh {
                    expected_revision: saved.running_revision,
                    document: Some("socks5://user:hidden-password@127.0.0.1:1080".into()),
                },
            )
            .await
            .unwrap();
        let overview = client.subscriptions().await.unwrap();
        let body = serde_json::to_string(&overview).unwrap();
        assert!(!body.contains("hidden-token"));
        assert!(!body.contains("hidden-password"));
        let nodes = client
            .nodes(&hps_domain::NodeQuery {
                provider: Some("subscription:fixture".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(nodes.nodes.len(), 1);
        let node = &nodes.nodes[0];
        assert!(node.udp_supported);
        assert!(node.eligible);
        let controlled = client
            .node_action(&hps_domain::NodeActionInput {
                id: node.id.clone(),
                action: "disable".into(),
                expected_revision: refreshed.running_revision,
                expected_control_version: 0,
                lease_id: None,
                batch_id: None,
                provider_id: node.provider_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            controlled.control.unwrap().state,
            hps_domain::NodeOperationState::Disabled
        );
        assert!(
            client
                .refresh_subscription(
                    "fixture",
                    &SubscriptionRefresh {
                        expected_revision: saved.running_revision,
                        document: Some("http://127.0.0.1:8080".into())
                    }
                )
                .await
                .is_err()
        );
        client
            .delete_subscription("fixture", refreshed.running_revision)
            .await
            .unwrap();
        assert!(
            client
                .subscriptions()
                .await
                .unwrap()
                .subscriptions
                .is_empty()
        );
        service.shutdown().await.unwrap();
        task.abort();
    }
}
