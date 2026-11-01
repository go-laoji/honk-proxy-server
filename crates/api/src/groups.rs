use super::*;
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/groups", get(overview))
        .route("/api/v1/groups/simulate", post(simulate))
        .route("/api/v1/groups/save", post(save))
        .route("/api/v1/groups/upsert", post(upsert))
        .route("/api/v1/groups/delete", post(delete))
        .route("/api/v1/groups/pin", post(pin))
        .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024))
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
async fn overview(
    State(state): State<Arc<AppState>>,
) -> ApiResult<hps_domain::GroupsOverview<hps_config::SelectionConfig>> {
    Ok(Json(application(&state)?.groups_overview()))
}
async fn simulate(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_domain::SelectionQuery>,
) -> ApiResult<hps_domain::SelectionDecision> {
    application(&state)?
        .simulate_group(input)
        .map(Json)
        .map_err(application_error)
}
async fn save(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::SelectionSave>,
) -> ApiResult<hps_domain::ConfigActivationResult> {
    application(&state)?
        .edit_groups(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn upsert(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::GroupUpsert>,
) -> ApiResult<hps_domain::ConfigActivationResult> {
    application(&state)?
        .upsert_group(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn delete(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::GroupDelete>,
) -> ApiResult<hps_domain::ConfigActivationResult> {
    application(&state)?
        .delete_group(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn pin(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::GroupPin>,
) -> ApiResult<hps_domain::ConfigActivationResult> {
    application(&state)?
        .pin_group(input)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_config::{GroupConfig, GroupDelete, GroupPin, GroupUpsert, SelectionSave};
    use hps_domain::SelectionQuery;
    #[tokio::test]
    async fn shared_client_crud_pin_preview_references_and_cas_use_actual_runtime() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        for id in ["a", "b"] {
            config.proxy.upstreams.push(hps_config::UpstreamConfig {
                id: id.into(),
                protocol: hps_config::ProxyProtocol::Http,
                host: "127.0.0.1".into(),
                port: 9,
                enabled: true,
                credentials: None,
                expires_at_unix: None,
                ca_certificate: None,
                tls_server_name: None,
            });
        }
        let application = Supervisor::boot(
            hps_storage::Store::open(None, config).unwrap(),
            hps_application::StartupOverrides::default(),
        )
        .await
        .unwrap();
        application.confirm_startup().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(AppState::with_application(application.clone()));
        let server = tokio::spawn(async move {
            axum::serve(listener, crate::router(state)).await.unwrap();
        });
        let client = hps_client::HpsClient::new(
            &format!("http://{address}"),
            std::time::Duration::from_secs(3),
        )
        .unwrap();
        assert!(
            client
                .groups()
                .await
                .unwrap()
                .configuration
                .groups
                .is_empty()
        );
        let group = GroupConfig {
            id: "g".into(),
            include_all: true,
            ..Default::default()
        };
        let input = GroupUpsert {
            group: group.clone(),
            expected_revision: 1,
        };
        assert_eq!(
            client.upsert_group(&input).await.unwrap().running_revision,
            2
        );
        assert!(matches!(
            client.upsert_group(&input).await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        let mut policy = client.groups().await.unwrap().configuration;
        policy.default_group = Some("g".into());
        assert_eq!(
            client
                .save_groups(&SelectionSave {
                    configuration: policy.clone(),
                    expected_revision: 2
                })
                .await
                .unwrap()
                .running_revision,
            3
        );
        assert_eq!(
            client
                .pin_group(&GroupPin {
                    group_id: "g".into(),
                    node_id: Some("b".into()),
                    fallback_nodes: vec![],
                    expected_revision: 3
                })
                .await
                .unwrap()
                .running_revision,
            4
        );
        for _ in 0..3 {
            let d = client
                .simulate_group(&SelectionQuery::default())
                .await
                .unwrap();
            assert_eq!(d.selected.as_deref(), Some("b"));
            assert_eq!(d.reason, "pinned");
            assert_eq!(d.sessions, 0);
        }
        assert!(client.groups().await.unwrap().recent_choices.is_empty());
        assert!(matches!(
            client
                .pin_group(&GroupPin {
                    group_id: "g".into(),
                    node_id: Some("missing".into()),
                    fallback_nodes: vec![],
                    expected_revision: 4
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        assert_eq!(client.groups().await.unwrap().running_revision, 4);
        assert_eq!(
            client
                .pin_group(&GroupPin {
                    group_id: "g".into(),
                    node_id: None,
                    fallback_nodes: vec![],
                    expected_revision: 4
                })
                .await
                .unwrap()
                .running_revision,
            5
        );
        assert!(matches!(
            client
                .delete_group(&GroupDelete {
                    group_id: "g".into(),
                    expected_revision: 5
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        policy.default_group = None;
        assert_eq!(
            client
                .save_groups(&SelectionSave {
                    configuration: policy,
                    expected_revision: 5
                })
                .await
                .unwrap()
                .running_revision,
            6
        );
        assert_eq!(
            client
                .delete_group(&GroupDelete {
                    group_id: "g".into(),
                    expected_revision: 6
                })
                .await
                .unwrap()
                .running_revision,
            7
        );
        assert!(matches!(
            client
                .delete_group(&GroupDelete {
                    group_id: "g".into(),
                    expected_revision: 7
                })
                .await,
            Err(hps_client::ClientError::Api { status: 404, .. })
        ));
        assert!(matches!(
            client
                .simulate_group(&SelectionQuery {
                    limit: Some(201),
                    ..Default::default()
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        let d = client
            .simulate_group(&SelectionQuery {
                transport: hps_domain::SelectionTransport::Udp,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(d.eligible, 0);
        let first = GroupUpsert {
            group: group.clone(),
            expected_revision: 7,
        };
        let second = GroupUpsert {
            group,
            expected_revision: 7,
        };
        let (a, b) = tokio::join!(client.upsert_group(&first), client.upsert_group(&second));
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert_eq!(client.groups().await.unwrap().running_revision, 8);
        server.abort();
        application.shutdown().await.unwrap();
    }
}
