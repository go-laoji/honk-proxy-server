use super::*;
use axum::extract::Query;
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/rules", get(overview))
        .route("/api/v1/rules/export", get(export))
        .route(
            "/api/v1/rules/import/preview",
            post(import_preview).layer(axum::extract::DefaultBodyLimit::max(1024 * 1024)),
        )
        .route(
            "/api/v1/rules/import/apply",
            post(import).layer(axum::extract::DefaultBodyLimit::max(1024 * 1024)),
        )
        .route("/api/v1/rules/save", post(save))
        .route("/api/v1/rules/upsert", post(upsert))
        .route("/api/v1/rules/delete", post(delete))
        .route("/api/v1/rules/toggle", post(toggle))
        .route("/api/v1/rules/reorder", post(reorder))
        .route("/api/v1/rules/apply", post(apply))
        .route("/api/v1/rules/simulate", post(simulate))
        .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024))
}
async fn export(
    State(state): State<Arc<AppState>>,
    Query(query): Query<View>,
) -> ApiResult<hps_config::RuleDocument> {
    application(&state)?
        .export_rules(query.revision)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn import_preview(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::RuleImportInput>,
) -> ApiResult<hps_domain::RuleImportPreview> {
    application(&state)?
        .preview_rule_import(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn import(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::RuleImportInput>,
) -> ApiResult<hps_domain::ConfigRevision> {
    application(&state)?
        .import_rules(input)
        .await
        .map(Json)
        .map_err(application_error)
}
fn application(state: &AppState) -> Result<&Arc<Supervisor>, (StatusCode, Json<ApiError>)> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct View {
    revision: Option<i64>,
}
async fn overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<View>,
) -> ApiResult<hps_domain::RulesOverview<hps_config::RoutingConfig>> {
    application(&state)?
        .rules_overview(query.revision)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn save(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::RulesSave>,
) -> ApiResult<hps_domain::ConfigRevision> {
    application(&state)?
        .save_rules(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn upsert(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::RuleUpsert>,
) -> ApiResult<hps_domain::ConfigRevision> {
    application(&state)?
        .upsert_rule(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn delete(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::RuleDelete>,
) -> ApiResult<hps_domain::ConfigRevision> {
    application(&state)?
        .delete_rule(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn toggle(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::RuleToggle>,
) -> ApiResult<hps_domain::ConfigRevision> {
    application(&state)?
        .toggle_rule(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn reorder(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_config::RuleOrder>,
) -> ApiResult<hps_domain::ConfigRevision> {
    application(&state)?
        .reorder_rules(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn apply(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_domain::ConfigActivate>,
) -> ApiResult<hps_domain::ConfigActivationResult> {
    application(&state)?
        .apply_rules(input)
        .await
        .map(Json)
        .map_err(application_error)
}
async fn simulate(
    State(state): State<Arc<AppState>>,
    Json(input): Json<hps_domain::RulesSimulationInput>,
) -> ApiResult<hps_domain::RouteSimulation> {
    application(&state)?
        .simulate_rules(input)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn versioned_rule_import_requires_preview_checksum_and_never_activates_its_candidate() {
        use super::*;
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
        let client = hps_client::HpsClient::new(
            &format!("http://{address}"),
            std::time::Duration::from_secs(3),
        )
        .unwrap();
        let mut document = client.export_rules(None).await.unwrap();
        assert_eq!(document.format_version, 1);
        document.configuration.rules = vec![hps_config::RuleConfig {
            id: "imported".into(),
            enabled: true,
            conditions: Default::default(),
            action: hps_domain::RouteAction::Reject,
        }];
        let mut input = hps_config::RuleImportInput {
            document,
            expected_revision: 1,
            checksum: None,
        };
        let preview = client.preview_rule_import(&input).await.unwrap();
        assert!(preview.validation.valid);
        assert_eq!(preview.rule_count, 1);
        assert!(matches!(
            client.import_rules(&input).await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        input.checksum = Some(preview.checksum);
        let candidate = client.import_rules(&input).await.unwrap();
        assert_eq!(candidate.id, 2);
        assert_eq!(client.rules(None).await.unwrap().running_revision, 1);
        assert_eq!(client.config_workspace().await.unwrap().desired_revision, 1);
        assert_eq!(
            client
                .export_rules(Some(2))
                .await
                .unwrap()
                .configuration
                .rules[0]
                .id,
            "imported"
        );
        client
            .apply_rules(&hps_domain::ConfigActivate {
                revision: 2,
                expected_revision: 1,
                defer_until_restart: false,
            })
            .await
            .unwrap();
        assert!(matches!(
            client.import_rules(&input).await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        input.expected_revision = 2;
        input.document.format_version = 99;
        assert!(matches!(
            client.preview_rule_import(&input).await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        input.document.format_version = 1;
        input.document.configuration.rules[0].action = hps_domain::RouteAction::Node {
            id: "missing".into(),
        };
        assert!(
            !client
                .preview_rule_import(&input)
                .await
                .unwrap()
                .validation
                .valid
        );
        assert!(matches!(
            client.import_rules(&input).await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        let oversized = reqwest::Client::new()
            .post(format!("http://{address}/api/v1/rules/import/preview"))
            .header("content-type", "application/json")
            .body(" ".repeat(1024 * 1024 + 1))
            .send()
            .await
            .unwrap();
        assert_eq!(oversized.status(), 413);
        service.shutdown().await.unwrap();
        server.abort();
    }
    use super::*;
    use hps_config::{
        RoutingConfig, RuleConditions, RuleConfig, RuleDelete, RuleOrder, RuleToggle, RuleUpsert,
        RulesSave,
    };
    use hps_domain::{
        ConfigActivate, FlowOrigin, RouteAction, RoutingContext, RulesSimulationInput,
        SelectionTransport,
    };
    fn rule(id: &str, all: bool) -> RuleConfig {
        RuleConfig {
            id: id.into(),
            enabled: true,
            conditions: if all {
                RuleConditions::default()
            } else {
                RuleConditions {
                    domains: vec!["blocked.example".into()],
                    ..Default::default()
                }
            },
            action: if all {
                RouteAction::Direct
            } else {
                RouteAction::Reject
            },
        }
    }
    fn query(revision: Option<i64>) -> RulesSimulationInput {
        RulesSimulationInput {
            revision,
            context: RoutingContext {
                destination_host: "blocked.example".into(),
                destination_port: 443,
                transport: SelectionTransport::Tcp,
                source_ip: None,
                source_port: None,
                inbound: None,
                inbound_protocol: None,
                observed_domains: vec![],
                origin: FlowOrigin::Explicit,
                process: None,
            },
        }
    }
    #[tokio::test]
    async fn actual_rule_client_separates_candidates_apply_simulation_crud_order_and_cas() {
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
        let client = hps_client::HpsClient::new(
            &format!("http://{address}"),
            std::time::Duration::from_secs(3),
        )
        .unwrap();
        let view = client.rules(None).await.unwrap();
        assert_eq!(view.running_revision, 1);
        assert!(!view.capabilities.local_process_available);
        assert!(!view.capabilities.kernel_rules_ready);
        let saved = client
            .save_rules(&RulesSave {
                configuration: RoutingConfig {
                    default_action: RouteAction::Direct,
                    ..Default::default()
                },
                expected_revision: 1,
                base_revision: None,
            })
            .await
            .unwrap();
        assert_eq!(saved.id, 2);
        assert_eq!(client.rules(None).await.unwrap().running_revision, 1);
        assert_eq!(
            client
                .simulate_rules(&query(None))
                .await
                .unwrap()
                .decision
                .action,
            RouteAction::Default
        );
        assert_eq!(
            client
                .simulate_rules(&query(Some(2)))
                .await
                .unwrap()
                .decision
                .action,
            RouteAction::Direct
        );
        assert_eq!(
            client
                .apply_rules(&ConfigActivate {
                    revision: 2,
                    expected_revision: 1,
                    defer_until_restart: false
                })
                .await
                .unwrap()
                .running_revision,
            2
        );
        let a = client
            .upsert_rule(&RuleUpsert {
                rule: rule("deny", false),
                expected_revision: 2,
                base_revision: None,
                index: None,
            })
            .await
            .unwrap();
        let b = client
            .upsert_rule(&RuleUpsert {
                rule: rule("allow", true),
                expected_revision: 2,
                base_revision: Some(a.id),
                index: None,
            })
            .await
            .unwrap();
        let c = client
            .reorder_rules(&RuleOrder {
                ids: vec!["allow".into(), "deny".into()],
                expected_revision: 2,
                base_revision: Some(b.id),
            })
            .await
            .unwrap();
        assert_eq!(
            client
                .simulate_rules(&query(Some(c.id)))
                .await
                .unwrap()
                .decision
                .rule_id
                .as_deref(),
            Some("allow")
        );
        let d = client
            .reorder_rules(&RuleOrder {
                ids: vec!["deny".into(), "allow".into()],
                expected_revision: 2,
                base_revision: Some(c.id),
            })
            .await
            .unwrap();
        assert_eq!(
            client
                .simulate_rules(&query(Some(d.id)))
                .await
                .unwrap()
                .decision
                .action,
            RouteAction::Reject
        );
        assert!(matches!(
            client
                .reorder_rules(&RuleOrder {
                    ids: vec!["deny".into(), "deny".into()],
                    expected_revision: 2,
                    base_revision: Some(d.id)
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        let disabled = client
            .toggle_rule(&RuleToggle {
                id: "deny".into(),
                enabled: false,
                expected_revision: 2,
                base_revision: Some(d.id),
            })
            .await
            .unwrap();
        assert_eq!(
            client
                .simulate_rules(&query(Some(disabled.id)))
                .await
                .unwrap()
                .decision
                .rule_id
                .as_deref(),
            Some("allow")
        );
        let deleted = client
            .delete_rule(&RuleDelete {
                id: "allow".into(),
                expected_revision: 2,
                base_revision: Some(disabled.id),
            })
            .await
            .unwrap();
        assert_eq!(
            client.rules(None).await.unwrap().configuration.rules.len(),
            0
        );
        let applied = client
            .apply_rules(&ConfigActivate {
                revision: deleted.id,
                expected_revision: 2,
                defer_until_restart: false,
            })
            .await
            .unwrap();
        assert_eq!(applied.running_revision, deleted.id);
        assert!(matches!(
            client
                .upsert_rule(&RuleUpsert {
                    rule: rule("stale", false),
                    expected_revision: 2,
                    base_revision: None,
                    index: None
                })
                .await,
            Err(hps_client::ClientError::Api { status: 409, .. })
        ));
        assert!(matches!(
            client
                .delete_rule(&RuleDelete {
                    id: "missing".into(),
                    expected_revision: deleted.id,
                    base_revision: None
                })
                .await,
            Err(hps_client::ClientError::Api { status: 404, .. })
        ));
        let mut process = rule("process", false);
        process.conditions.process = Some(hps_config::ProcessCondition {
            command: Some("browser".into()),
            ..Default::default()
        });
        assert!(matches!(
            client
                .upsert_rule(&RuleUpsert {
                    rule: process.clone(),
                    expected_revision: deleted.id,
                    base_revision: None,
                    index: None
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        process.enabled = false;
        let proc = client
            .upsert_rule(&RuleUpsert {
                rule: process,
                expected_revision: deleted.id,
                base_revision: None,
                index: None,
            })
            .await
            .unwrap();
        assert!(matches!(
            client
                .toggle_rule(&RuleToggle {
                    id: "process".into(),
                    enabled: true,
                    expected_revision: deleted.id,
                    base_revision: Some(proc.id)
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        assert!(matches!(
            client
                .save_rules(&RulesSave {
                    configuration: RoutingConfig {
                        default_action: RouteAction::Node {
                            id: "lease:999999".into()
                        },
                        ..Default::default()
                    },
                    expected_revision: deleted.id,
                    base_revision: None
                })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        assert_eq!(
            client.rules(None).await.unwrap().running_revision,
            deleted.id
        );
        server.abort();
        service.shutdown().await.unwrap();
    }
}
