use super::*;
use hps_config::DnsCaptureConfig;
use hps_dns::{Resolver, server::DnsServer};

pub(super) struct CaptureRuntime {
    fingerprint: String,
    specification: DnsCaptureConfig,
    servers: Vec<Arc<DnsServer>>,
    #[cfg(target_os = "linux")]
    handle: Arc<hps_platform::dns_capture::DnsCapture>,
}
pub(super) enum PreparedCapture {
    Disabled,
    #[cfg(target_os = "linux")]
    Existing(Arc<CaptureRuntime>),
    #[cfg(target_os = "linux")]
    New(hps_platform::dns_capture::PreparedCapture, DnsCaptureConfig),
}
impl PreparedCapture {
    pub fn prepare(
        config: &DaemonConfig,
        previous: Option<&RuntimeSnapshot>,
    ) -> Result<Self, ApplicationError> {
        let Some(specification) = &config.dns.capture else {
            return Ok(Self::Disabled);
        };
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (specification, previous);
            Err(ApplicationError::Capture(
                "DNS eBPF capture requires Linux; no hooks were loaded".into(),
            ))
        }
        #[cfg(target_os = "linux")]
        {
            let prepared = hps_platform::dns_capture::PreparedCapture::read(
                &specification.object,
                &specification.cgroup,
                specification.port,
            )
            .map_err(ApplicationError::Capture)?;
            if let Some(old) = previous.and_then(|p| p.capture.as_ref())
                && old.specification == *specification
                && old.fingerprint == prepared.fingerprint
            {
                return Ok(Self::Existing(old.clone()));
            }
            prepared.verify().map_err(ApplicationError::Capture)?;
            Ok(Self::New(prepared, specification.clone()))
        }
    }
    pub fn requires_restart(&self, old: Option<&Arc<CaptureRuntime>>) -> bool {
        let Some(_old) = old else { return false };
        match self {
            Self::Disabled => true,
            #[cfg(target_os = "linux")]
            Self::Existing(value) => !Arc::ptr_eq(_old, value),
            #[cfg(target_os = "linux")]
            Self::New(_, _) => true,
        }
    }
    pub async fn ready(
        self,
        resolver: &Resolver,
    ) -> Result<Option<Arc<CaptureRuntime>>, ApplicationError> {
        match self {
            Self::Disabled => {
                let _ = resolver;
                Ok(None)
            }
            #[cfg(target_os = "linux")]
            Self::Existing(value) => Ok(Some(value)),
            #[cfg(target_os = "linux")]
            Self::New(prepared, specification) => {
                let fingerprint = prepared.fingerprint.clone();
                let port = specification.port;
                // Binds and kernel attachment are completed with the redirect map
                // disabled, before the configuration head is changed.
                let v4 = DnsServer::bind_capture(
                    SocketAddr::from(([0, 0, 0, 0], port)),
                    resolver.clone(),
                )
                .await
                .map_err(|_| EngineError::Bind)?;
                let v6 = DnsServer::bind_capture(
                    SocketAddr::new(std::net::Ipv6Addr::UNSPECIFIED.into(), port),
                    resolver.clone(),
                )
                .await
                .map_err(|_| EngineError::Bind)?;
                let handle = tokio::task::spawn_blocking(move || prepared.load())
                    .await
                    .map_err(|_| ApplicationError::Worker)?
                    .map_err(ApplicationError::Capture)?;
                Ok(Some(Arc::new(CaptureRuntime {
                    fingerprint,
                    specification,
                    servers: vec![v4, v6],
                    handle: Arc::new(handle),
                })))
            }
        }
    }
}
impl CaptureRuntime {
    pub fn start(&self, resolver: &Resolver) -> Result<(), ApplicationError> {
        for server in &self.servers {
            server.replace_resolver(resolver.clone());
            server.start();
        }
        #[cfg(target_os = "linux")]
        self.handle.activate().map_err(ApplicationError::Capture)?;
        Ok(())
    }
    pub fn status(&self) -> Result<hps_domain::DnsCaptureStatus, ApplicationError> {
        #[cfg(target_os = "linux")]
        {
            self.handle.status().map_err(ApplicationError::Capture)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (&self.fingerprint, &self.specification);
            Err(ApplicationError::Capture(
                "DNS capture requires Linux".into(),
            ))
        }
    }
    pub async fn shutdown(&self) -> Result<(), ApplicationError> {
        #[cfg(target_os = "linux")]
        {
            let handle = self.handle.clone();
            tokio::task::spawn_blocking(move || handle.shutdown())
                .await
                .map_err(|_| ApplicationError::Worker)?
                .map_err(ApplicationError::Capture)?;
        }
        for server in &self.servers {
            server.shutdown().await;
        }
        Ok(())
    }
}
