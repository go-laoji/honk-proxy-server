use hps_domain::{ApiError, Capabilities, ServiceStatus, VersionInfo};
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use std::time::Duration;
mod diagnostics;
mod subscriptions;
mod events;
pub use events::EventSubscription;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid background task ID or epoch")]
    InvalidTaskId,
    #[error("invalid event cursor")]
    InvalidEventCursor,
    #[error("event stream returned an invalid, oversized or out-of-order frame")]
    EventProtocol,
    #[error("invalid connection observation ID")]
    InvalidConnectionId,
    #[error("endpoint must be an HTTP(S) URL without credentials, query or fragment")]
    InvalidEndpoint,
    #[error("CA certificate bundle is empty or invalid")]
    InvalidCertificate,
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("HTTP {status}: {error:?}")]
    Api { status: u16, error: ApiError },
}

#[derive(Clone)]
pub struct HpsClient {
    http: Client,
    stream_http: Client,
    endpoint: Url,
}

impl HpsClient {
    pub fn new(endpoint: &str, timeout: Duration) -> Result<Self, ClientError> {
        Self::with_ca(endpoint, timeout, None)
    }
    pub fn with_ca(
        endpoint: &str,
        timeout: Duration,
        certificate: Option<&[u8]>,
    ) -> Result<Self, ClientError> {
        let mut endpoint = Url::parse(endpoint).map_err(|_| ClientError::InvalidEndpoint)?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(ClientError::InvalidEndpoint);
        }
        if !endpoint.path().ends_with('/') {
            endpoint.set_path(&format!("{}/", endpoint.path()));
        }
        let mut http = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy();
        let mut stream_http = Client::builder()
            .connect_timeout(timeout)
            .read_timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy();
        if let Some(certificate) = certificate {
            let certificates = reqwest::Certificate::from_pem_bundle(certificate)
                .map_err(|_| ClientError::InvalidCertificate)?;
            if certificates.is_empty() {
                return Err(ClientError::InvalidCertificate);
            }
            for certificate in certificates {
                stream_http = stream_http.add_root_certificate(certificate.clone());
                http = http.add_root_certificate(certificate);
            }
        }
        let http = http.build()?;
        let stream_http = stream_http.build()?;
        Ok(Self {
            http,
            stream_http,
            endpoint,
        })
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub fn console_url(&self) -> Url {
        self.endpoint
            .join("console/index.html")
            .expect("constant relative URL")
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let url = self
            .endpoint
            .join(path)
            .map_err(|_| ClientError::InvalidEndpoint)?;
        let response = self.http.get(url).send().await?;
        Self::decode(response).await
    }

    pub async fn post_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let url = self
            .endpoint
            .join(path)
            .map_err(|_| ClientError::InvalidEndpoint)?;
        Self::decode(self.http.post(url).send().await?).await
    }
    pub async fn post_body<T: DeserializeOwned, B: serde::Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        let url = self
            .endpoint
            .join(path)
            .map_err(|_| ClientError::InvalidEndpoint)?;
        Self::decode(self.http.post(url).json(body).send().await?).await
    }
    pub async fn health(
        &self,
        after: Option<&str>,
        limit: u32,
    ) -> Result<hps_domain::HealthOverview<hps_config::HealthConfig>, ClientError> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("limit", &limit.to_string());
        if let Some(after) = after {
            query.append_pair("after", after);
        }
        self.get_json(&format!("api/v1/health?{}", query.finish()))
            .await
    }
    pub async fn health_history(
        &self,
        input: &hps_domain::HealthHistoryQuery,
    ) -> Result<hps_domain::HealthHistory, ClientError> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if let Some(node) = &input.node {
            query.append_pair("node", node);
        }
        if let Some(since) = input.since_unix {
            query.append_pair("since_unix", &since.to_string());
        }
        if let Some(before) = input.before {
            query.append_pair("before", &before.to_string());
        }
        query.append_pair("limit", &input.limit.unwrap_or(100).to_string());
        self.get_json(&format!("api/v1/health/history?{}", query.finish()))
            .await
    }
    pub async fn observations(
        &self,
        query: &hps_domain::ObservationQuery,
    ) -> Result<hps_domain::ObservationOverview, ClientError> {
        self.get_json(&format!(
            "api/v1/observability?window_secs={}",
            query.window_secs
        ))
        .await
    }
    pub async fn export_rules(
        &self,
        revision: Option<i64>,
    ) -> Result<hps_config::RuleDocument, ClientError> {
        self.get_json(&revision.map_or("api/v1/rules/export".into(), |r| {
            format!("api/v1/rules/export?revision={r}")
        }))
        .await
    }
    pub async fn preview_rule_import(
        &self,
        input: &hps_config::RuleImportInput,
    ) -> Result<hps_domain::RuleImportPreview, ClientError> {
        self.post_body("api/v1/rules/import/preview", input).await
    }
    pub async fn import_rules(
        &self,
        input: &hps_config::RuleImportInput,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/rules/import/apply", input).await
    }
    pub async fn dns(
        &self,
        after: Option<&str>,
        limit: u32,
    ) -> Result<hps_dns::Overview<hps_config::DnsConfig>, ClientError> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("limit", &limit.to_string());
        if let Some(after) = after {
            query.append_pair("after", after);
        }
        self.get_json(&format!("api/v1/dns?{}", query.finish()))
            .await
    }
    pub async fn groups(
        &self,
    ) -> Result<hps_domain::GroupsOverview<hps_config::SelectionConfig>, ClientError> {
        self.get_json("api/v1/groups").await
    }
    pub async fn rules(
        &self,
        revision: Option<i64>,
    ) -> Result<hps_domain::RulesOverview<hps_config::RoutingConfig>, ClientError> {
        self.get_json(&revision.map_or_else(
            || "api/v1/rules".into(),
            |id| format!("api/v1/rules?revision={id}"),
        ))
        .await
    }
    pub async fn save_rules(
        &self,
        input: &hps_config::RulesSave,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/rules/save", input).await
    }
    pub async fn upsert_rule(
        &self,
        input: &hps_config::RuleUpsert,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/rules/upsert", input).await
    }
    pub async fn delete_rule(
        &self,
        input: &hps_config::RuleDelete,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/rules/delete", input).await
    }
    pub async fn toggle_rule(
        &self,
        input: &hps_config::RuleToggle,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/rules/toggle", input).await
    }
    pub async fn reorder_rules(
        &self,
        input: &hps_config::RuleOrder,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/rules/reorder", input).await
    }
    pub async fn apply_rules(
        &self,
        input: &hps_domain::ConfigActivate,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/rules/apply", input).await
    }
    pub async fn simulate_rules(
        &self,
        input: &hps_domain::RulesSimulationInput,
    ) -> Result<hps_domain::RouteSimulation, ClientError> {
        self.post_body("api/v1/rules/simulate", input).await
    }
    pub async fn simulate_group(
        &self,
        input: &hps_domain::SelectionQuery,
    ) -> Result<hps_domain::SelectionDecision, ClientError> {
        self.post_body("api/v1/groups/simulate", input).await
    }
    pub async fn nodes(
        &self,
        query: &hps_domain::NodeQuery,
    ) -> Result<hps_domain::NodesOverview, ClientError> {
        let mut url = self
            .endpoint
            .join("api/v1/nodes")
            .map_err(|_| ClientError::InvalidEndpoint)?;
        if let serde_json::Value::Object(fields) = serde_json::to_value(query).expect("node query")
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in fields {
                if !value.is_null() {
                    pairs.append_pair(
                        &key,
                        &match value {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        },
                    );
                }
            }
        }
        Self::decode(self.http.get(url).send().await?).await
    }
    pub async fn connections(
        &self,
        query: &hps_domain::ConnectionQuery,
    ) -> Result<hps_domain::ConnectionsOverview, ClientError> {
        let mut url = self
            .endpoint
            .join("api/v1/connections")
            .map_err(|_| ClientError::InvalidEndpoint)?;
        if let serde_json::Value::Object(fields) =
            serde_json::to_value(query).expect("connection query")
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in fields {
                if !value.is_null() {
                    pairs.append_pair(
                        &key,
                        &match value {
                            serde_json::Value::String(s) => s,
                            other => other.to_string(),
                        },
                    );
                }
            }
        }
        Self::decode(self.http.get(url).send().await?).await
    }
    pub async fn connection_summary(&self) -> Result<hps_domain::ConnectionSummary, ClientError> {
        self.get_json("api/v1/connections/summary").await
    }
    pub async fn connection(&self, id: &str) -> Result<hps_domain::ConnectionInfo, ClientError> {
        if !hps_domain::valid_connection_id(id) {
            return Err(ClientError::InvalidConnectionId);
        }
        self.get_json(&format!("api/v1/connections/{id}")).await
    }
    pub async fn disconnect_connection(
        &self,
        input: &hps_domain::ConnectionDisconnectInput,
    ) -> Result<hps_domain::ConnectionDisconnectResult, ClientError> {
        self.post_body("api/v1/connections/disconnect", input).await
    }
    pub async fn preview_disconnect_connections(
        &self,
        input: &hps_domain::ConnectionDisconnectPreviewInput,
    ) -> Result<hps_domain::ConnectionDisconnectPreview, ClientError> {
        self.post_body("api/v1/connections/disconnect/preview", input)
            .await
    }
    pub async fn confirm_disconnect_connections(
        &self,
        input: &hps_domain::ConnectionDisconnectConfirm,
    ) -> Result<hps_domain::ConnectionDisconnectResult, ClientError> {
        self.post_body("api/v1/connections/disconnect/confirm", input)
            .await
    }
    pub async fn node(&self, id: &str) -> Result<hps_domain::NodesOverview, ClientError> {
        if !hps_config::valid_node_id(id) {
            return Err(ClientError::InvalidEndpoint);
        }
        self.get_json(&format!("api/v1/nodes/{id}")).await
    }
    pub async fn node_action(
        &self,
        input: &hps_domain::NodeActionInput,
    ) -> Result<hps_domain::NodeActionResult, ClientError> {
        self.post_body("api/v1/nodes/action", input).await
    }
    pub async fn preview_nodes(
        &self,
        input: &hps_domain::NodeImportInput,
    ) -> Result<hps_domain::NodeImportPreview, ClientError> {
        self.post_body("api/v1/nodes/import/preview", input).await
    }
    pub async fn import_nodes(
        &self,
        input: &hps_domain::NodeImportInput,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/nodes/import/apply", input).await
    }
    pub async fn save_groups(
        &self,
        input: &hps_config::SelectionSave,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/groups/save", input).await
    }
    pub async fn upsert_group(
        &self,
        input: &hps_config::GroupUpsert,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/groups/upsert", input).await
    }
    pub async fn delete_group(
        &self,
        input: &hps_config::GroupDelete,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/groups/delete", input).await
    }
    pub async fn pin_group(
        &self,
        input: &hps_config::GroupPin,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/groups/pin", input).await
    }
    pub async fn resolve_dns(
        &self,
        input: &hps_dns::LookupInput,
    ) -> Result<hps_dns::LookupResult, ClientError> {
        self.post_body("api/v1/dns/resolve", input).await
    }
    pub async fn clear_dns(
        &self,
        input: &hps_dns::ClearInput,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_body("api/v1/dns/cache/clear", input).await
    }
    pub async fn dns_context(
        &self,
        ip: std::net::IpAddr,
    ) -> Result<hps_dns::DomainContext, ClientError> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("ip", &ip.to_string());
        self.get_json(&format!("api/v1/dns/context?{}", query.finish()))
            .await
    }
    pub async fn save_dns(
        &self,
        input: &hps_config::DnsSave,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/dns/save", input).await
    }
    pub async fn probe_health(
        &self,
        input: &hps_domain::HealthProbeInput,
    ) -> Result<hps_domain::HealthTask, ClientError> {
        self.post_body("api/v1/health/probe", input).await
    }
    pub async fn health_task(&self, id: i64) -> Result<hps_domain::HealthTask, ClientError> {
        self.get_json(&format!("api/v1/health/tasks/{id}")).await
    }
    pub async fn save_health(
        &self,
        input: &hps_config::HealthSave,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/health/save", input).await
    }
    pub async fn providers(
        &self,
    ) -> Result<Vec<hps_domain::ProviderView<hps_config::ProviderConfig>>, ClientError> {
        self.get_json("api/v1/providers").await
    }
    pub async fn provider(
        &self,
        id: &str,
    ) -> Result<hps_domain::ProviderDetail<hps_config::ProviderConfig>, ClientError> {
        self.get_json(&format!("api/v1/providers/{}", segment(id)?))
            .await
    }
    pub async fn save_provider(
        &self,
        configuration: hps_config::ProviderConfig,
        expected_revision: i64,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body(
            &format!("api/v1/providers/{}/save", segment(&configuration.id)?),
            &hps_config::ProviderSave {
                configuration,
                expected_revision,
            },
        )
        .await
    }
    pub async fn delete_provider(
        &self,
        id: &str,
        expected_revision: i64,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body(
            &format!("api/v1/providers/{}/delete", segment(id)?),
            &hps_domain::ExpectedRevision { expected_revision },
        )
        .await
    }
    pub async fn extract_provider(
        &self,
        id: &str,
        request_id: String,
    ) -> Result<hps_domain::ExtractionTask, ClientError> {
        self.post_body(
            &format!("api/v1/providers/{}/extract", segment(id)?),
            &hps_domain::ExtractionInput { request_id },
        )
        .await
    }
    pub async fn provider_task(&self, id: i64) -> Result<hps_domain::ExtractionTask, ClientError> {
        self.get_json(&format!("api/v1/provider-tasks/{id}")).await
    }
    pub async fn provider_leases(
        &self,
        id: &str,
        before: Option<i64>,
    ) -> Result<Vec<hps_domain::LeaseInfo>, ClientError> {
        self.get_json(&format!(
            "api/v1/providers/{}/leases{}",
            segment(id)?,
            before.map_or_else(String::new, |id| format!("?before={id}"))
        ))
        .await
    }
    pub async fn provider_controller(
        &self,
        id: &str,
    ) -> Result<hps_domain::ProviderControllerStatus, ClientError> {
        self.get_json(&format!("api/v1/providers/{}/controller", segment(id)?))
            .await
    }
    pub async fn pause_provider(
        &self,
        id: &str,
        paused: bool,
    ) -> Result<hps_domain::ProviderControllerStatus, ClientError> {
        self.post_json(&format!(
            "api/v1/providers/{}/controller/{}",
            segment(id)?,
            if paused { "pause" } else { "resume" }
        ))
        .await
    }
    pub async fn evaluate_provider(
        &self,
        id: &str,
    ) -> Result<hps_domain::ControllerEvaluation, ClientError> {
        self.post_json(&format!(
            "api/v1/providers/{}/controller/evaluate",
            segment(id)?
        ))
        .await
    }
    pub async fn provider_diagnostic(
        &self,
        id: &str,
        kind: &str,
    ) -> Result<serde_json::Value, ClientError> {
        if !["quota", "whitelist"].contains(&kind) {
            return Err(ClientError::InvalidEndpoint);
        }
        self.post_json(&format!(
            "api/v1/providers/{}/diagnose/{kind}",
            segment(id)?
        ))
        .await
    }
    pub async fn config_workspace(&self) -> Result<hps_domain::ConfigWorkspace, ClientError> {
        self.get_json("api/v1/config/workspace").await
    }
    pub async fn validate_config(
        &self,
        input: &hps_domain::ConfigInput,
    ) -> Result<hps_domain::ConfigValidation, ClientError> {
        self.post_body("api/v1/config/validate", input).await
    }
    pub async fn save_draft(
        &self,
        input: &hps_domain::ConfigInput,
    ) -> Result<hps_domain::ConfigDraft, ClientError> {
        self.post_body("api/v1/config/draft", input).await
    }
    pub async fn create_revision(
        &self,
        input: &hps_domain::ConfigInput,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/config/revisions", input).await
    }
    pub async fn activate_config(
        &self,
        input: &hps_domain::ConfigActivate,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/config/activate", input).await
    }
    pub async fn rollback_config(
        &self,
        input: &hps_domain::ConfigActivate,
    ) -> Result<hps_domain::ConfigActivationResult, ClientError> {
        self.post_body("api/v1/config/rollback", input).await
    }
    pub async fn export_config(
        &self,
        revision: Option<i64>,
    ) -> Result<hps_domain::ConfigExport, ClientError> {
        let path = revision.map_or_else(
            || "api/v1/config/export".into(),
            |revision| format!("api/v1/config/revisions/{revision}"),
        );
        self.get_json(&path).await
    }

    async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, ClientError> {
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error = response
                .json::<ApiError>()
                .await
                .unwrap_or_else(|_| ApiError {
                    code: "invalid_server_response".into(),
                    message: "target returned an unexpected response".into(),
                    request_id: String::new(),
                    retryable: status >= 500,
                });
            return Err(ClientError::Api { status, error });
        }
        Ok(response.json().await?)
    }

    pub async fn status(&self) -> Result<ServiceStatus, ClientError> {
        self.get_json("api/v1/status").await
    }

    pub async fn version(&self) -> Result<VersionInfo, ClientError> {
        self.get_json("api/v1/version").await
    }

    pub async fn capabilities(&self) -> Result<Capabilities, ClientError> {
        self.get_json("api/v1/capabilities").await
    }

    pub async fn config(&self) -> Result<hps_config::DaemonConfig, ClientError> {
        self.get_json("api/v1/config").await
    }

    pub async fn linux_preflight(&self) -> Result<hps_domain::LinuxPreflight, ClientError> {
        self.get_json("api/v1/linux/preflight").await
    }
    pub async fn linux_capture(
        &self,
    ) -> Result<hps_domain::LinuxCaptureOverview<hps_config::LinuxCaptureConfig>, ClientError> {
        self.get_json("api/v1/linux/capture").await
    }
    pub async fn control_linux_capture(
        &self,
        input: &hps_domain::LinuxCaptureRequest,
    ) -> Result<hps_domain::LinuxCaptureStatus, ClientError> {
        self.post_body("api/v1/linux/capture/control", input).await
    }
    pub async fn save_linux_capture(
        &self,
        input: &hps_config::LinuxCaptureSave,
    ) -> Result<hps_domain::ConfigRevision, ClientError> {
        self.post_body("api/v1/linux/capture/save", input).await
    }
    pub async fn inbounds(&self) -> Result<hps_domain::ProxyStatus, ClientError> {
        self.get_json("api/v1/inbounds").await
    }
    pub async fn start_inbound(&self, id: &str) -> Result<hps_domain::InboundStatus, ClientError> {
        self.inbound_action(id, "start").await
    }
    pub async fn stop_inbound(&self, id: &str) -> Result<hps_domain::InboundStatus, ClientError> {
        self.inbound_action(id, "stop").await
    }
    async fn inbound_action(
        &self,
        id: &str,
        action: &str,
    ) -> Result<hps_domain::InboundStatus, ClientError> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(ClientError::InvalidEndpoint);
        }
        self.post_json(&format!("api/v1/inbounds/{id}/{action}"))
            .await
    }
}

fn segment(id: &str) -> Result<&str, ClientError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        Err(ClientError::InvalidEndpoint)
    } else {
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_validation_and_subpath_preservation() {
        for endpoint in [
            "file:///tmp/test",
            "http://user:pass@localhost",
            "http://localhost/?token=x",
        ] {
            assert!(HpsClient::new(endpoint, Duration::from_secs(1)).is_err());
        }
        let client = HpsClient::new("https://localhost/hps", Duration::from_secs(1)).unwrap();
        assert_eq!(
            client.console_url().as_str(),
            "https://localhost/hps/console/index.html"
        );
    }
}
