//! Shared daemon transparent capture; desktop and CLI use this same service.
use super::*;
use hps_config::LinuxCaptureConfig;
#[cfg(target_os = "linux")]
mod linux;

pub(super) struct Controller {
    specification: LinuxCaptureConfig,
    fingerprint: String,
    #[cfg(target_os = "linux")]
    runtime: linux::Controller,
}
pub(super) enum Prepared {
    Disabled,
    #[cfg(target_os = "linux")]
    Existing(Arc<Controller>),
    #[cfg(target_os = "linux")]
    New(Box<hps_platform::transparent::PreparedCapture>),
}
impl Prepared {
    pub fn prepare(
        config: &DaemonConfig,
        previous: Option<&RuntimeSnapshot>,
    ) -> Result<Self, ApplicationError> {
        let Some(specification) = &config.linux_capture else {
            return Ok(Self::Disabled);
        };
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (specification, previous);
            Err(ApplicationError::Capture(
                "transparent eBPF capture requires Linux".into(),
            ))
        }
        #[cfg(target_os = "linux")]
        {
            let mut effective = specification.clone();
            if config.dns.capture.is_some() {
                for alias in ["127.77.0.0/16", "fd77:6870:7300::/112"] {
                    let net: ipnet::IpNet = alias.parse().unwrap();
                    if !effective
                        .bypass_cidrs
                        .iter()
                        .filter_map(|cidr| cidr.parse::<ipnet::IpNet>().ok())
                        .any(|existing| existing.contains(&net))
                    {
                        effective.bypass_cidrs.push(alias.into());
                    }
                }
            }
            let prepared = hps_platform::transparent::PreparedCapture::read(&effective)
                .map_err(ApplicationError::Capture)?;
            if let Some(old) = previous.and_then(|p| p.transparent.as_ref())
                && old.specification == prepared.specification
                && old.fingerprint == prepared.fingerprint
            {
                return Ok(Self::Existing(old.clone()));
            }
            prepared.verify().map_err(ApplicationError::Capture)?;
            Ok(Self::New(Box::new(prepared)))
        }
    }
    pub fn requires_restart(&self, old: Option<&Arc<Controller>>) -> bool {
        let Some(_old) = old else { return false };
        match self {
            Self::Disabled => true,
            #[cfg(target_os = "linux")]
            Self::Existing(value) => !Arc::ptr_eq(value, _old),
            #[cfg(target_os = "linux")]
            Self::New(_) => true,
        }
    }
    pub async fn ready(self) -> Result<Option<Arc<Controller>>, ApplicationError> {
        match self {
            Self::Disabled => Ok(None),
            #[cfg(target_os = "linux")]
            Self::Existing(value) => Ok(Some(value)),
            #[cfg(target_os = "linux")]
            Self::New(prepared) => {
                let specification = prepared.specification.clone();
                let fingerprint = prepared.fingerprint.clone();
                let runtime = linux::Controller::prepare(*prepared)
                    .await
                    .map_err(ApplicationError::Capture)?;
                Ok(Some(Arc::new(Controller {
                    specification,
                    fingerprint,
                    runtime,
                })))
            }
        }
    }
}
impl Controller {
    pub async fn status(&self, revision: i64) -> hps_domain::LinuxCaptureStatus {
        let mut view = hps_domain::LinuxCaptureStatus::unconfigured(revision);
        view.configured = true;
        view.object_path = Some(self.specification.object.display().to_string());
        view.object_fingerprint = Some(self.fingerprint.clone());
        view.scope = Some(hps_domain::CaptureScope {
            local_cgroup: self
                .specification
                .local_cgroup
                .as_ref()
                .map(|p| p.display().to_string()),
            lan_interfaces: self.specification.lan_interfaces.clone(),
            ipv4: self.specification.ipv4,
            ipv6: self.specification.ipv6,
            tcp: self.specification.tcp,
            udp: self.specification.udp,
        });
        #[cfg(target_os = "linux")]
        {
            self.runtime.status(view).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            view
        }
    }

    pub async fn publish(&self, engine: Arc<Engine>) -> Result<(), ApplicationError> {
        #[cfg(target_os = "linux")]
        {
            self.runtime
                .publish(engine)
                .await
                .map_err(ApplicationError::Capture)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (engine, &self.specification, &self.fingerprint);
            Err(ApplicationError::Capture(
                "transparent capture requires Linux".into(),
            ))
        }
    }
    pub async fn shutdown(&self) -> Result<(), ApplicationError> {
        #[cfg(target_os = "linux")]
        {
            self.runtime
                .shutdown()
                .await
                .map_err(ApplicationError::Capture)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(())
        }
    }
    pub fn pinned(&self) -> Vec<i64> {
        #[cfg(target_os = "linux")]
        {
            self.runtime.pinned()
        }
        #[cfg(not(target_os = "linux"))]
        {
            vec![]
        }
    }
}
