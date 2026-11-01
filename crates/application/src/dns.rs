use super::*;
use hps_config::{DnsConfig, DnsSave, ProxyConfig};
use hps_dns::{Resolver, server::DnsServer};
use hps_engine::{dial::ProbePlan, target::Target};
use hps_net::{BoxIo, DnsConnector, NetworkError, NetworkFuture};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

struct StaticDnsEgress {
    plans: BTreeMap<String, Vec<ProbePlan>>,
}
impl DnsConnector for StaticDnsEgress {
    fn connect(
        &self,
        id: String,
        target: SocketAddr,
        deadline: tokio::time::Instant,
    ) -> NetworkFuture<'_, BoxIo> {
        Box::pin(async move {
            let plans = self
                .plans
                .get(&id)
                .ok_or(NetworkError("dns_missing_static_proxy"))?;
            let target = Target::new(&target.ip().to_string(), target.port())
                .map_err(|_| NetworkError("dns_invalid_target"))?;
            for plan in plans {
                if plan
                    .remaining()
                    .is_some_and(|d| d.as_secs() <= plan.minimum_lifetime())
                {
                    continue;
                }
                if let Ok(connected) = plan.connect_until(&target, false, deadline).await {
                    return Ok(connected.io);
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
            Err(NetworkError("dns_static_proxy_failed"))
        })
    }
    fn fingerprint(&self, id: &str) -> Option<String> {
        let mut hash = Sha256::new();
        for plan in self.plans.get(id)? {
            hash.update(plan.identity());
        }
        Some(format!("{:x}", hash.finalize()))
    }
}
pub(super) fn prepare(
    config: &DaemonConfig,
    previous: Option<&RuntimeSnapshot>,
) -> Result<Resolver, ApplicationError> {
    let mut plans = BTreeMap::<String, Vec<ProbePlan>>::new();
    for up in &config.dns.upstreams {
        let Some(id) = &up.proxy_id else { continue };
        if plans.contains_key(id) {
            continue;
        }
        let original = config
            .proxy
            .upstreams
            .iter()
            .find(|p| &p.id == id && p.enabled)
            .ok_or(ApplicationError::Invalid)?;
        let mut endpoints = vec![];
        for ip in original
            .host
            .parse::<std::net::IpAddr>()
            .ok()
            .into_iter()
            .chain(up.proxy_bootstrap_ips.iter().copied())
        {
            let mut endpoint = original.clone();
            if endpoint.protocol == hps_config::ProxyProtocol::Https
                && endpoint.tls_server_name.is_none()
            {
                endpoint.tls_server_name = Some(endpoint.host.clone());
            }
            endpoint.host = ip.to_string();
            endpoints.push(endpoint);
        }
        // Each IP uses its own prepared plan; no hostname lookup, dynamic pool,
        // health selection or demand signal can lead back into DNS bootstrap.
        let mut choices = vec![];
        for endpoint in endpoints {
            let engine = Engine::new(ProxyConfig {
                upstreams: vec![endpoint],
                limits: config.proxy.limits.clone(),
                ..Default::default()
            })?;
            choices.extend(engine.probe_plans());
        }
        plans.insert(id.clone(), choices);
    }
    let resolver = Resolver::prepare(
        config.dns.clone(),
        &config.proxy,
        Some(Arc::new(StaticDnsEgress { plans })),
    )
    .map_err(|_| ApplicationError::Invalid)?;
    Ok(previous
        .filter(|old| old.dns.fingerprint() == resolver.fingerprint())
        .map_or(resolver, |old| old.dns.clone()))
}
pub(super) async fn listener(
    resolver: &Resolver,
    previous: Option<&RuntimeSnapshot>,
) -> Result<Option<Arc<DnsServer>>, ApplicationError> {
    let config = resolver.configuration();
    let Some(address) = config.listen.filter(|_| config.enabled) else {
        return Ok(None);
    };
    if let Some(old) = previous
        .and_then(|p| p.dns_server.as_ref())
        .filter(|s| s.configured_address() == address)
    {
        return Ok(Some(old.clone()));
    }
    DnsServer::bind(address, resolver.clone())
        .await
        .map(Some)
        .map_err(|_| EngineError::Bind.into())
}

pub type DnsOverview = hps_dns::Overview<DnsConfig>;
impl Supervisor {
    pub fn dns_overview(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<DnsOverview, ApplicationError> {
        let current = self.current();
        Ok(DnsOverview {
            running_revision: current.revision,
            configuration: current.config.dns.clone(),
            listener: current.dns_server.as_ref().map(|s| s.local_address()),
            cache: current.dns.cache(after, limit)?,
            linux_capture_active: false,
        })
    }
    pub async fn edit_dns(
        self: &Arc<Self>,
        input: DnsSave,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let current = self.current();
        let workspace = self.store.workspace().await?;
        if current.revision != input.expected_revision
            || workspace.desired.revision.id != input.expected_revision
        {
            return Err(StorageError::Conflict.into());
        }
        let mut config = current.base_config.clone();
        config.dns = input.configuration;
        let revision = self
            .create_revision(ConfigInput {
                toml: toml::to_string(&config).map_err(|_| ApplicationError::Invalid)?,
                expected_revision: input.expected_revision,
                expected_draft_version: None,
                note: "DNS policy".into(),
                use_draft_credentials: false,
            })
            .await?;
        self.activate(ConfigActivate {
            revision: revision.id,
            expected_revision: input.expected_revision,
            defer_until_restart: false,
        })
        .await
    }
    pub fn name_resolver(&self) -> Option<Arc<dyn hps_net::NameResolver>> {
        let current = self.current();
        current
            .config
            .dns
            .enabled
            .then(|| Arc::new(current.dns.clone()) as Arc<dyn hps_net::NameResolver>)
    }
}
