use hps_domain::{ApiError, Capabilities, ServiceStatus, VersionInfo};
use reqwest::{Client, Url};
use serde::de::DeserializeOwned;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("endpoint must be an HTTP(S) URL without credentials, query or fragment")]
    InvalidEndpoint,
    #[error("request failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("HTTP {status}: {error:?}")]
    Api { status: u16, error: ApiError },
}

#[derive(Clone)]
pub struct HpsClient {
    http: Client,
    endpoint: Url,
}

impl HpsClient {
    pub fn new(endpoint: &str, timeout: Duration) -> Result<Self, ClientError> {
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
        let http = Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()?;
        Ok(Self { http, endpoint })
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
