use super::*;
use hps_config::ProviderConfig;
use hps_domain::{ExtractionInput, ExtractionTask, LeaseInfo, ProviderDetail, ProviderView};
use hps_provider::{FetchError, ProviderClient};
use std::collections::{HashMap, HashSet};
impl Supervisor {
    pub(crate) async fn refresh_pool(&self) -> Result<(), ApplicationError> {
        let current = self.current();
        let allowed: HashMap<_, _> = current
            .config
            .providers
            .iter()
            .filter(|p| p.enabled)
            .map(|p| (p.id.clone(), hps_provider::fingerprint(p)))
            .collect();
        let mut leases = self.store.active_leases().await?;
        let healthy = current
            .engine
            .probe_plans()
            .into_iter()
            .filter(|p| current.engine.health_usable(p.identity()))
            .map(|p| {
                (
                    p.provider_id().map(str::to_owned),
                    health_endpoint_key(p.config()),
                )
            })
            .collect::<HashSet<_>>();
        leases.sort_by_cached_key(|lease| {
            std::cmp::Reverse((
                healthy.contains(&(
                    Some(lease.provider_id.clone()),
                    health_endpoint_key(&lease.candidate.upstream),
                )),
                lease.candidate.expires_at,
                lease.id,
            ))
        });
        let mut seen = HashSet::new();
        let mut counts = HashMap::<String, u32>::new();
        let nodes = leases
            .into_iter()
            .filter(|lease| allowed.get(&lease.provider_id) == Some(&lease.fingerprint))
            .filter(|lease| {
                seen.insert((
                    lease.provider_id.clone(),
                    format!("{:?}", lease.candidate.upstream.protocol),
                    lease.candidate.upstream.host.clone(),
                    lease.candidate.upstream.port,
                ))
            })
            .filter(|lease| {
                let provider = current
                    .config
                    .providers
                    .iter()
                    .find(|p| p.id == lease.provider_id)
                    .expect("allowed provider");
                let count = counts.entry(lease.provider_id.clone()).or_default();
                if provider.refill.enabled && *count >= provider.refill.max_nodes {
                    false
                } else {
                    *count += 1;
                    true
                }
            })
            .map(|lease| hps_engine::pool::PoolNode {
                provider_id: lease.provider_id.clone(),
                fingerprint: lease.fingerprint,
                min_remaining_secs: current
                    .config
                    .providers
                    .iter()
                    .find(|p| p.id == lease.provider_id)
                    .map_or(0, |p| {
                        if p.refill.enabled {
                            p.min_remaining_secs.max(p.refill.retire_before_secs)
                        } else {
                            p.min_remaining_secs
                        }
                    }),
                upstream: lease.candidate.upstream,
            })
            .collect();
        let pool = current.engine.dynamic_pool();
        tokio::task::spawn_blocking(move || pool.replace(nodes))
            .await
            .map_err(|_| ApplicationError::Worker)?
            .map_err(|_| ApplicationError::Invalid)
    }
    pub async fn providers(&self) -> Result<Vec<ProviderView<ProviderConfig>>, ApplicationError> {
        let current = self.current();
        let leases = self.store.active_leases().await?;
        let mut providers = vec![];
        for config in &current.config.providers {
            let fp = hps_provider::fingerprint(config);
            let active = if config.enabled {
                leases
                    .iter()
                    .filter(|l| l.provider_id == config.id && l.fingerprint == fp)
                    .count()
            } else {
                0
            };
            let tasks = self.store.extraction_tasks(Some(config.id.clone())).await?;
            providers.push(ProviderView {
                configuration: config.redacted(),
                active_leases: active,
                latest_task: tasks.into_iter().next(),
                health_state: if !config.enabled {
                    "disabled"
                } else if !current.config.health.enabled {
                    "checking_disabled"
                } else if current.engine.probe_plans().iter().any(|p| {
                    p.provider_id() == Some(config.id.as_str())
                        && current.engine.health_usable(p.identity())
                }) {
                    "healthy"
                } else if active == 0 {
                    "empty"
                } else {
                    "awaiting_healthy_node"
                }
                .into(),
            });
        }
        Ok(providers)
    }
    pub async fn provider_detail(
        &self,
        id: &str,
    ) -> Result<ProviderDetail<ProviderConfig>, ApplicationError> {
        let provider = self
            .providers()
            .await?
            .into_iter()
            .find(|p| p.configuration.id == id)
            .ok_or(ApplicationError::ProviderUnavailable)?;
        Ok(ProviderDetail {
            provider,
            tasks: self.store.extraction_tasks(Some(id.into())).await?,
            leases: self.provider_leases(Some(id.into()), None, 50).await?,
        })
    }
    pub async fn provider_leases(
        &self,
        id: Option<String>,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<LeaseInfo>, ApplicationError> {
        let mut leases = self.store.provider_leases(id, before, limit).await?;
        let current = self.current();
        for lease in &mut leases {
            if lease.state != "active" {
                continue;
            }
            match current
                .config
                .providers
                .iter()
                .find(|p| p.id == lease.provider_id)
            {
                None => lease.state = "source_deleted".into(),
                Some(p) if !p.enabled => lease.state = "source_disabled".into(),
                Some(p) if hps_provider::fingerprint(p) != lease.config_fingerprint => {
                    lease.state = "source_changed".into()
                }
                _ => {}
            }
        }
        Ok(leases)
    }
    pub async fn extraction_task(&self, id: i64) -> Result<ExtractionTask, ApplicationError> {
        Ok(self.store.extraction_task(id).await?)
    }
    pub async fn extract(
        self: &Arc<Self>,
        id: String,
        input: ExtractionInput,
    ) -> Result<ExtractionTask, ApplicationError> {
        // Accept and spawn in an owned task so disconnecting cannot leave a new task stranded.
        let service = self.clone();
        tokio::spawn(async move { service.extract_inner(id, input).await })
            .await
            .map_err(|_| ApplicationError::Worker)?
    }
    async fn extract_inner(
        self: &Arc<Self>,
        id: String,
        input: ExtractionInput,
    ) -> Result<ExtractionTask, ApplicationError> {
        let _operation = self.operation.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        let config = self
            .current()
            .config
            .providers
            .iter()
            .find(|p| p.id == id && p.enabled)
            .cloned()
            .ok_or(ApplicationError::ProviderUnavailable)?;
        self.accept_extraction_locked(config, input.request_id, "manual")
            .await
    }
    /// The caller holds operation; both API extraction and the controller use this path.
    pub(crate) async fn accept_extraction_locked(
        self: &Arc<Self>,
        config: ProviderConfig,
        request_id: String,
        trigger: &str,
    ) -> Result<ExtractionTask, ApplicationError> {
        if let Some(task) = self
            .store
            .existing_extraction(config.id.clone(), request_id.clone())
            .await?
        {
            return Ok(task);
        }
        let logical = self
            .current()
            .engine
            .dynamic_pool()
            .logical_now()
            .min(i64::MAX as u64) as i64;
        if logical > hps_provider::unix_now().saturating_add(5) {
            return Err(ApplicationError::ClockRollback);
        }
        let permit = self
            .provider_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::ExtractionCapacity)?;
        let client = ProviderClient::new(&config)?;
        self.store.observe_lease_clock(logical).await?;
        let (task, new) = self
            .store
            .begin_extraction_with_trigger(
                config.clone(),
                request_id,
                hps_provider::fingerprint(&config),
                trigger.into(),
            )
            .await?;
        if !new {
            return Ok(task);
        }
        let service = self.clone();
        let task_id = task.id;
        self.provider_tasks.spawn(async move {
            let _permit=permit;
            if service.store.extraction_requesting(task_id).await.is_err() {return;}
            let result=tokio::select! {
                _=service.provider_cancel.cancelled()=>Err(FetchError{code:"shutdown_outcome_unknown",outcome_unknown:true,retry_after_secs:None}),
                result=client.fetch(&config,hps_provider::unix_now())=>result,
            };
            let _operation=service.operation.lock().await;
            match result {
                Err(error)=>{let _=service.store.fail_extraction(task_id,error).await;},
                Ok(result)=>{
                    let publish=!service.closed.load(std::sync::atomic::Ordering::Acquire) && service.current().config.providers.iter().any(|p|p.enabled && p.id==config.id && hps_provider::fingerprint(p)==hps_provider::fingerprint(&config));
                    if service.store.finish_extraction(task_id,config,result,publish).await.is_ok() {let _=service.refresh_pool().await;} else {let _=service.store.fail_extraction(task_id,FetchError{code:"lease_commit_outcome_unknown",outcome_unknown:true,retry_after_secs:None}).await;}
                },
            }
        });
        Ok(task)
    }
    pub async fn provider_diagnostic(
        &self,
        id: &str,
        kind: &str,
    ) -> Result<serde_json::Value, ApplicationError> {
        let config = self
            .current()
            .config
            .providers
            .iter()
            .find(|p| p.id == id && p.enabled)
            .cloned()
            .ok_or(ApplicationError::ProviderUnavailable)?;
        let _permit = self
            .provider_slots
            .try_acquire()
            .map_err(|_| ApplicationError::Worker)?;
        Ok(ProviderClient::new(&config)?
            .diagnostic(&config, kind)
            .await?)
    }
    /// Small convenience API over immutable configuration revisions, with the same CAS.
    pub async fn edit_provider(
        self: &Arc<Self>,
        id: String,
        provider: Option<ProviderConfig>,
        expected: i64,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let workspace = self.store.workspace().await?;
        let current = self.current();
        if current.revision != expected || workspace.desired.revision.id != expected {
            return Err(StorageError::Conflict.into());
        }
        let mut config = current.base_config.clone();
        if let Some(mut provider) = provider {
            if provider.id != id {
                return Err(ApplicationError::Invalid);
            }
            provider
                .restore_redacted(config.providers.iter().find(|p| p.id == id))
                .map_err(|_| ApplicationError::Invalid)?;
            config.providers.retain(|p| p.id != id);
            config.providers.push(provider);
        } else {
            let before = config.providers.len();
            config.providers.retain(|p| p.id != id);
            if config.providers.len() == before {
                return Err(ApplicationError::ProviderUnavailable);
            }
        }
        let revision = self
            .create_revision(ConfigInput {
                toml: toml::to_string(&config).map_err(|_| ApplicationError::Invalid)?,
                expected_revision: expected,
                expected_draft_version: None,
                note: format!("provider {id}"),
                use_draft_credentials: false,
            })
            .await?;
        self.activate(ConfigActivate {
            revision: revision.id,
            expected_revision: expected,
            defer_until_restart: false,
        })
        .await
    }
}

fn health_endpoint_key(upstream: &hps_config::UpstreamConfig) -> String {
    let mut config = upstream.clone();
    config.id.clear();
    config.enabled = true;
    config.expires_at_unix = None;
    serde_json::to_string(&config).expect("upstream serialization")
}
