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
