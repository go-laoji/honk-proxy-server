use super::*;
use axum::extract::Query;
pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/v1/observability", get(overview))
}
async fn overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<hps_domain::ObservationQuery>,
) -> ApiResult<hps_domain::ObservationOverview> {
    state
        .application
        .as_ref()
        .ok_or_else(|| application_error(ApplicationError::Worker))?
        .observation_overview(query)
        .await
        .map(Json)
        .map_err(application_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn actual_rejected_business_flow_has_exact_hits_and_only_bounded_metadata_without_auth() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.proxy.routing.default_action = hps_domain::RouteAction::Reject;
        config.proxy.inbounds.push(hps_config::InboundConfig {
            id: "observe".into(),
            protocol: hps_config::ProxyProtocol::Http,
            listen: "127.0.0.1:0".parse().unwrap(),
            enabled: true,
            tls: None,
            max_connections: 8,
            bytes_per_second: 0,
        });
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
        let first = client
            .observations(&hps_domain::ObservationQuery::default())
            .await
            .unwrap();
        assert!(first.rule_hits.counters.is_empty());
        let inlet = service.current().engine.status().inbounds[0]
            .listen
            .clone()
            .unwrap();
        let response = reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(format!("http://{inlet}")).unwrap())
            .build()
            .unwrap()
            .get("http://origin.invalid/private-business-path")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        drop(response);
        let observed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let value = client
                    .observations(&hps_domain::ObservationQuery { window_secs: 60 })
                    .await
                    .unwrap();
                if !value.correlations.groups.is_empty() {
                    break value;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            observed
                .rule_hits
                .counters
                .iter()
                .map(|c| c.hits)
                .sum::<u64>(),
            1
        );
        assert_eq!(
            observed.correlations.groups[0].destination_host,
            "origin.invalid"
        );
        assert!(
            !serde_json::to_string(&observed)
                .unwrap()
                .contains("private-business-path")
        );
        assert!(
            observed.samples.len() <= observed.sample_limit && observed.resets_on_daemon_restart
        );
        assert!(matches!(
            client
                .observations(&hps_domain::ObservationQuery { window_secs: 3601 })
                .await,
            Err(hps_client::ClientError::Api { status: 422, .. })
        ));
        service.shutdown().await.unwrap();
        server.abort();
    }
}
