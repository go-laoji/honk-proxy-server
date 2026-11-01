mod controller;
mod health;
mod providers;
use arc_swap::ArcSwap;
use hps_config::DaemonConfig;
use hps_domain::{
    ConfigActivate, ConfigActivationResult, ConfigChange, ConfigDiagnostic, ConfigDraft,
    ConfigInput, ConfigRevision, ConfigValidation, ConfigWorkspace, InboundStatus,
};
use hps_engine::runtime::{Engine, EngineError};
use hps_storage::{StorageError, Store};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug, thiserror::Error)]
pub enum ApplicationError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("configuration is invalid; run validation for field diagnostics")]
    Invalid,
    #[error("too many draining generations; close old connections before activating")]
    GenerationBudget,
    #[error("runtime preparation could not finish")]
    Worker,
    #[error("provider not found or disabled")]
    ProviderUnavailable,
    #[error("maximum concurrent supplier fetches reached")]
    ExtractionCapacity,
    #[error("system clock is behind the running lease clock; correct it before extracting")]
    ClockRollback,
    #[error("provider operation failed: {0}")]
    Provider(#[from] hps_provider::FetchError),
}
#[derive(Default, Clone)]
pub struct StartupOverrides {
    pub listen: Option<SocketAddr>,
    pub instance_name: Option<String>,
}
impl StartupOverrides {
    fn apply(&self, config: &mut DaemonConfig) {
        if let Some(listen) = self.listen {
            config.management_listen = listen;
        }
        if let Some(name) = &self.instance_name {
            config.instance_name = name.clone();
        }
    }
    fn fields(&self) -> Vec<String> {
        let mut fields = vec![];
        if self.listen.is_some() {
            fields.push("management_listen".into());
        }
        if self.instance_name.is_some() {
            fields.push("instance_name".into());
        }
        fields
    }
}
pub struct RuntimeSnapshot {
    pub revision: i64,
    pub config: DaemonConfig,
    pub engine: Arc<Engine>,
    pub management_tls: Option<hps_engine::tls::PreparedServer>,
    pub health_profiles: Vec<hps_health::PreparedProfile>,
    base_config: DaemonConfig,
}
pub struct Supervisor {
    store: Store,
    current: ArcSwap<RuntimeSnapshot>,
    operation: tokio::sync::Mutex<()>,
    retired: Mutex<Vec<Arc<Engine>>>,
    overrides: StartupOverrides,
    path_base: PathBuf,
    closed: std::sync::atomic::AtomicBool,
    pending_restart: Mutex<Option<(i64, Vec<String>)>>,
    provider_tasks: tokio_util::task::TaskTracker,
    provider_cancel: tokio_util::sync::CancellationToken,
    provider_slots: Arc<tokio::sync::Semaphore>,
    controller_started: std::sync::atomic::AtomicBool,
    health: health::HealthRuntime,
}
impl Supervisor {
    pub async fn boot(
        store: Store,
        overrides: StartupOverrides,
    ) -> Result<Arc<Self>, ApplicationError> {
        let workspace = store.workspace().await?;
        let base_config = workspace.desired.config;
        let mut config = base_config.clone();
        overrides.apply(&mut config);
        let preparing = config.clone();
        let prepared = tokio::task::spawn_blocking(move || prepare_engine(&preparing, None))
            .await
            .map_err(|_| ApplicationError::Worker)??;
        let engine = prepared.engine;
        engine.set_generation(workspace.desired.revision.id);
        engine.prepare_enabled(None).await?;
        engine.start_enabled().await?;
        let path_base = store
            .database_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir().map_err(|_| ApplicationError::Worker)?);
        let service = Arc::new(Self {
            store,
            current: ArcSwap::from_pointee(RuntimeSnapshot {
                revision: workspace.desired.revision.id,
                config,
                base_config,
                engine,
                management_tls: prepared.management_tls,
                health_profiles: prepared.health_profiles,
            }),
            operation: tokio::sync::Mutex::new(()),
            retired: Mutex::new(vec![]),
            overrides,
            path_base,
            closed: std::sync::atomic::AtomicBool::new(false),
            pending_restart: Mutex::new(None),
            provider_tasks: tokio_util::task::TaskTracker::new(),
            provider_cancel: tokio_util::sync::CancellationToken::new(),
            provider_slots: Arc::new(tokio::sync::Semaphore::new(8)),
            controller_started: std::sync::atomic::AtomicBool::new(false),
            health: Default::default(),
        });
        service.load_health().await?;
        service.refresh_pool().await?;
        Ok(service)
    }
    pub fn current(&self) -> Arc<RuntimeSnapshot> {
        self.current.load_full()
    }
    pub async fn confirm_startup(self: &Arc<Self>) -> Result<(), ApplicationError> {
        self.store
            .mark_applied(self.current().revision, None)
            .await?;
        self.prune().await?;
        self.start_health_scheduler();
        self.start_pool_controller();
        Ok(())
    }
    pub async fn workspace(&self) -> Result<ConfigWorkspace, ApplicationError> {
        let workspace = self.store.workspace().await?;
        let current = self.current();
        let mut retired = self.retired.lock().unwrap();
        retired.retain(|engine| !engine.is_idle());
        let mut pending = restart_fields(&current.base_config, &workspace.desired.config);
        if let Some((revision, reasons)) = self.pending_restart.lock().unwrap().as_ref()
            && *revision == workspace.desired.revision.id
        {
            for reason in reasons {
                if !pending.contains(reason) {
                    pending.push(reason.clone());
                }
            }
        }
        Ok(ConfigWorkspace {
            format_version: 1,
            desired_revision: workspace.desired.revision.id,
            running_revision: current.revision,
            current_toml: export(&current.config)?,
            desired_toml: export(&workspace.desired.config)?,
            draft: ConfigDraft {
                version: workspace.draft.version,
                base_revision: workspace.draft.base_revision,
                toml: export(&workspace.draft.config)?,
                updated_at: workspace.draft.updated_at,
            },
            revisions: workspace.revisions.into_iter().map(revision_view).collect(),
            activations: workspace
                .activations
                .into_iter()
                .map(|activation| hps_domain::ConfigActivation {
                    id: activation.id,
                    revision: activation.revision,
                    previous_revision: activation.previous_revision,
                    state: activation.state,
                    created_at: activation.created_at,
                    error_code: activation.error_code,
                })
                .collect(),
            pending_restart: pending,
            storage_path: self
                .store
                .database_path
                .as_ref()
                .map(|path| path.display().to_string()),
            draining_generations: retired.len(),
            startup_overrides: self.overrides.fields(),
            management_certificate: current.management_tls.as_ref().map(|prepared| {
                let info = &prepared.certificate;
                let now = hps_storage::unix_now();
                hps_domain::CertificateStatus {
                    not_before_unix: info.not_before_unix,
                    not_after_unix: info.not_after_unix,
                    valid_now: now >= info.not_before_unix && now <= info.not_after_unix,
                }
            }),
        })
    }
    async fn parse(&self, input: &ConfigInput) -> Result<DaemonConfig, Vec<ConfigDiagnostic>> {
        let workspace = self
            .store
            .workspace()
            .await
            .map_err(|_| vec![diagnostic("storage", "configuration snapshot unavailable")])?;
        if workspace.desired.revision.id != input.expected_revision {
            return Err(vec![diagnostic(
                "expected_revision",
                "configuration changed; reload before editing",
            )]);
        }
        let mut config: DaemonConfig =
            toml::from_str(&input.toml).map_err(|error: toml::de::Error| {
                let (line, column) = error
                    .span()
                    .map(|span| {
                        let prefix = &input.toml[..span.start.min(input.toml.len())];
                        (
                            prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
                            prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
                        )
                    })
                    .unwrap_or((1, 1));
                vec![ConfigDiagnostic {
                    path: "toml".into(),
                    message: "invalid TOML syntax or field type".into(),
                    line: Some(line),
                    column: Some(column),
                }]
            })?;
        let reference = if input.use_draft_credentials {
            if input.expected_draft_version != Some(workspace.draft.version) {
                return Err(vec![diagnostic(
                    "expected_draft_version",
                    "shared draft changed; reload before referencing its credentials",
                )]);
            }
            &workspace.draft.config
        } else {
            &workspace.desired.config
        };
        config
            .restore_redacted(reference)
            .map_err(|error| vec![diagnostic("configuration", &error.to_string())])?;
        config.resolve_paths(&self.path_base);
        Ok(config)
    }
    pub async fn validate(&self, input: ConfigInput) -> ConfigValidation {
        let candidate = match self.parse(&input).await {
            Ok(candidate) => candidate,
            Err(diagnostics) => {
                return ConfigValidation {
                    valid: false,
                    diagnostics,
                    changes: vec![],
                    restart_required: vec![],
                };
            }
        };
        let current = self.current();
        let changes = changes(&current.config, &candidate);
        let restart_required = restart_fields(&current.base_config, &candidate);
        let result = tokio::task::spawn_blocking(move || prepare_engine(&candidate, None)).await;
        let diagnostics = match result {
            Ok(Ok(_)) => vec![],
            _ => vec![diagnostic(
                "tls_or_log_filter",
                "certificate/key/CA or log filter validation failed",
            )],
        };
        ConfigValidation {
            valid: diagnostics.is_empty(),
            diagnostics,
            changes,
            restart_required,
        }
    }
    pub async fn save_draft(&self, input: ConfigInput) -> Result<ConfigDraft, ApplicationError> {
        let _operation = self.operation.lock().await;
        let candidate = self
            .parse(&input)
            .await
            .map_err(|diagnostics| parse_error(&diagnostics))?;
        let expected = input
            .expected_draft_version
            .ok_or(ApplicationError::Invalid)?;
        let draft = self
            .store
            .save_draft(input.expected_revision, expected, candidate)
            .await?;
        Ok(ConfigDraft {
            version: draft.version,
            base_revision: draft.base_revision,
            toml: export(&draft.config)?,
            updated_at: draft.updated_at,
        })
    }
    pub async fn create_revision(
        &self,
        input: ConfigInput,
    ) -> Result<ConfigRevision, ApplicationError> {
        let _operation = self.operation.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        let workspace = self.store.workspace().await?;
        if workspace.desired.revision.id != input.expected_revision
            || input
                .expected_draft_version
                .is_some_and(|version| version != workspace.draft.version)
        {
            return Err(StorageError::Conflict.into());
        }
        let candidate = self
            .parse(&input)
            .await
            .map_err(|diagnostics| parse_error(&diagnostics))?;
        let preparing = candidate.clone();
        tokio::task::spawn_blocking(move || prepare_engine(&preparing, None))
            .await
            .map_err(|_| ApplicationError::Worker)??;
        let revision = revision_view(
            self.store
                .create_revision(input.expected_revision, candidate, input.note)
                .await?,
        );
        self.prune().await?;
        Ok(revision)
    }
    pub async fn activate(
        self: &Arc<Self>,
        input: ConfigActivate,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        // A caller disconnecting must not abandon a committed head halfway
        // through a generation handoff. The activation history is recoverable.
        let service = self.clone();
        tokio::spawn(async move { service.activate_inner(input).await })
            .await
            .map_err(|_| ApplicationError::Worker)?
    }
    async fn activate_inner(
        &self,
        input: ConfigActivate,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let _operation = self.operation.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        let workspace = self.store.workspace().await?;
        if workspace.desired.revision.id != input.expected_revision {
            return Err(StorageError::Conflict.into());
        }
        let target = self.store.revision(input.revision).await?;
        let current = self.current();
        let mut restart_required = restart_fields(&current.base_config, &target.config);
        let mut config = target.config.clone();
        self.overrides.apply(&mut config);
        let prepare = config.clone();
        let previous = current.engine.clone();
        let prepared =
            match tokio::task::spawn_blocking(move || prepare_engine(&prepare, Some(&previous)))
                .await
                .map_err(|_| ApplicationError::Worker)?
            {
                Ok(candidate) => candidate,
                Err(error) => {
                    self.store
                        .record_failure(
                            input.revision,
                            input.expected_revision,
                            "configuration_preparation_failed",
                        )
                        .await?;
                    return Err(error);
                }
            };
        if current
            .management_tls
            .as_ref()
            .map(|prepared| &prepared.certificate.fingerprint)
            != prepared
                .management_tls
                .as_ref()
                .map(|prepared| &prepared.certificate.fingerprint)
            && !restart_required
                .iter()
                .any(|field| field == "management_tls")
        {
            restart_required.push("management_tls.certificate_contents".into());
        }
        let candidate = prepared.engine;
        if !restart_required.is_empty() || input.defer_until_restart {
            let activation_id = self
                .store
                .activate(input.revision, input.expected_revision, true)
                .await?;
            let restart_required = if restart_required.is_empty() {
                vec!["explicit_restart_requested".into()]
            } else {
                restart_required
            };
            *self.pending_restart.lock().unwrap() =
                Some((input.revision, restart_required.clone()));
            return Ok(ConfigActivationResult {
                activation_id,
                desired_revision: input.revision,
                running_revision: current.revision,
                state: "pending_restart".into(),
                restart_required,
            });
        }
        if serde_json::to_value(&current.base_config.proxy)
            .map_err(|_| ApplicationError::Invalid)?
            == serde_json::to_value(&target.config.proxy).map_err(|_| ApplicationError::Invalid)?
            && serde_json::to_value(&current.base_config.providers)
                .map_err(|_| ApplicationError::Invalid)?
                == serde_json::to_value(&target.config.providers)
                    .map_err(|_| ApplicationError::Invalid)?
            && current
                .health_profiles
                .iter()
                .map(|p| &p.fingerprint)
                .eq(prepared.health_profiles.iter().map(|p| &p.fingerprint))
            && serde_json::to_value(&current.base_config.health)
                .map_err(|_| ApplicationError::Invalid)?
                == serde_json::to_value(&target.config.health)
                    .map_err(|_| ApplicationError::Invalid)?
        {
            let activation_id = self
                .store
                .activate(input.revision, input.expected_revision, false)
                .await?;
            self.current.store(Arc::new(RuntimeSnapshot {
                revision: input.revision,
                config,
                base_config: target.config,
                engine: current.engine.clone(),
                management_tls: prepared.management_tls,
                health_profiles: prepared.health_profiles,
            }));
            *self.pending_restart.lock().unwrap() = None;
            self.store
                .mark_applied(input.revision, Some(activation_id))
                .await?;
            self.prune().await?;
            return Ok(ConfigActivationResult {
                activation_id,
                desired_revision: input.revision,
                running_revision: input.revision,
                state: "applied".into(),
                restart_required: vec![],
            });
        }
        {
            let mut retired = self.retired.lock().unwrap();
            retired.retain(|engine| !engine.is_idle());
            if retired.len() >= 32 {
                return Err(ApplicationError::GenerationBudget);
            }
        }
        if let Err(error) = candidate.prepare_enabled(Some(&current.engine)).await {
            self.store
                .record_failure(
                    input.revision,
                    input.expected_revision,
                    "listen_preparation_failed",
                )
                .await?;
            return Err(error.into());
        }
        let activation_id = self
            .store
            .activate(input.revision, input.expected_revision, false)
            .await?;
        candidate.set_generation(input.revision);
        if let Err(error) = candidate.start_enabled().await {
            candidate.shutdown().await;
            let rollback = self
                .store
                .activate(current.revision, input.revision, false)
                .await?;
            self.store
                .mark_applied(current.revision, Some(rollback))
                .await?;
            return Err(error.into());
        }
        current.engine.drain_acceptors().await;
        self.current.store(Arc::new(RuntimeSnapshot {
            revision: input.revision,
            config,
            base_config: target.config,
            engine: candidate,
            management_tls: prepared.management_tls,
            health_profiles: prepared.health_profiles,
        }));
        self.refresh_pool().await?;
        self.retired.lock().unwrap().push(current.engine.clone());
        *self.pending_restart.lock().unwrap() = None;
        self.store
            .mark_applied(input.revision, Some(activation_id))
            .await?;
        self.prune().await?;
        Ok(ConfigActivationResult {
            activation_id,
            desired_revision: input.revision,
            running_revision: input.revision,
            state: "applied".into(),
            restart_required: vec![],
        })
    }
    pub async fn export_revision(&self, id: i64) -> Result<String, ApplicationError> {
        export(&self.store.revision(id).await?.config)
    }
    pub async fn rollback(
        self: &Arc<Self>,
        input: ConfigActivate,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        if self
            .store
            .revision(input.revision)
            .await?
            .revision
            .applied_at
            .is_none()
        {
            return Err(ApplicationError::Invalid);
        }
        self.activate(input).await
    }
    pub async fn start_inbound(&self, id: &str) -> Result<InboundStatus, ApplicationError> {
        let _operation = self.operation.lock().await;
        Ok(self.current().engine.start(id).await?)
    }
    pub async fn stop_inbound(&self, id: &str) -> Result<InboundStatus, ApplicationError> {
        let _operation = self.operation.lock().await;
        self.current().engine.stop(id).await?;
        let retired = self.retired.lock().unwrap().clone();
        for engine in retired {
            match engine.stop(id).await {
                Ok(_) | Err(EngineError::NotFound) => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.current()
            .engine
            .status()
            .inbounds
            .into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| EngineError::NotFound.into())
    }
    async fn prune(&self) -> Result<(), ApplicationError> {
        let current = self.current();
        let mut pinned = vec![current.revision, current.engine.generation()];
        {
            let mut retired = self.retired.lock().unwrap();
            retired.retain(|engine| !engine.is_idle());
            pinned.extend(retired.iter().map(|engine| engine.generation()));
        }
        self.store
            .prune_versions(current.config.retention.config_versions, pinned)
            .await?;
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<(), ApplicationError> {
        let _operation = self.operation.lock().await;
        if self.closed.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Ok(());
        }
        self.provider_cancel.cancel();
        drop(_operation);
        self.provider_tasks.close();
        self.provider_tasks.wait().await;
        self.current().engine.shutdown().await;
        let retired = std::mem::take(&mut *self.retired.lock().unwrap());
        for engine in retired {
            engine.shutdown().await;
        }
        self.store
            .observe_lease_clock(
                self.current()
                    .engine
                    .dynamic_pool()
                    .logical_now()
                    .min(i64::MAX as u64) as i64,
            )
            .await?;
        self.store.close().await?;
        Ok(())
    }
}
fn parse_error(diagnostics: &[ConfigDiagnostic]) -> ApplicationError {
    if diagnostics
        .iter()
        .any(|error| error.path.starts_with("expected_"))
    {
        StorageError::Conflict.into()
    } else {
        ApplicationError::Invalid
    }
}
fn diagnostic(path: &str, message: &str) -> ConfigDiagnostic {
    ConfigDiagnostic {
        path: path.into(),
        message: message.into(),
        line: None,
        column: None,
    }
}
fn revision_view(revision: hps_storage::Revision) -> ConfigRevision {
    ConfigRevision {
        id: revision.id,
        parent_id: revision.parent_id,
        checksum: revision.checksum,
        created_at: revision.created_at,
        note: revision.note,
        applied_at: revision.applied_at,
    }
}
fn export(config: &DaemonConfig) -> Result<String, ApplicationError> {
    toml::to_string_pretty(&config.redacted())
        .map(|body| {
            format!(
                "# honk-proxy-server configuration format_version=1; credentials redacted\n{body}"
            )
        })
        .map_err(|_| ApplicationError::Invalid)
}
struct PreparedRuntime {
    health_profiles: Vec<hps_health::PreparedProfile>,
    engine: Arc<Engine>,
    management_tls: Option<hps_engine::tls::PreparedServer>,
}
fn prepare_engine(
    config: &DaemonConfig,
    previous: Option<&Arc<Engine>>,
) -> Result<PreparedRuntime, ApplicationError> {
    tracing_subscriber::EnvFilter::try_new(&config.log_filter)
        .map_err(|_| ApplicationError::Invalid)?;
    let management_tls = if let Some(tls) = &config.management_tls {
        let prepared =
            hps_engine::tls::prepare_server(tls).map_err(|_| ApplicationError::Invalid)?;
        if !prepared.certificate.valid_now {
            return Err(ApplicationError::Invalid);
        }
        Some(prepared)
    } else {
        None
    };
    let engine = Engine::with_previous(config.proxy.clone(), previous)?;
    let health_profiles = if config.health.enabled {
        config
            .health
            .profiles
            .iter()
            .filter(|p| p.enabled && p.transport == hps_domain::HealthTransport::Tcp)
            .cloned()
            .map(hps_health::PreparedProfile::prepare)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ApplicationError::Invalid)?
    } else {
        vec![]
    };
    engine.configure_health(hps_engine::health::HealthPolicy {
        enabled: config.health.enabled,
        profiles: health_profiles
            .iter()
            .map(|p| (p.fingerprint.clone(), p.profile.transport, p.profile.family))
            .collect(),
        max_staleness_secs: config.health.max_staleness_secs,
        traffic_failure_threshold: config.health.traffic_failure_threshold,
    });
    engine.set_provider_sources(
        config
            .providers
            .iter()
            .filter(|p| p.enabled)
            .map(|p| (p.id.clone(), hps_provider::fingerprint(p)))
            .collect(),
    );
    if engine.status().inbounds.iter().any(|entry| {
        entry
            .certificate
            .as_ref()
            .is_some_and(|info| !info.valid_now)
    }) {
        return Err(ApplicationError::Invalid);
    }
    Ok(PreparedRuntime {
        health_profiles,
        engine,
        management_tls,
    })
}
fn restart_fields(current: &DaemonConfig, target: &DaemonConfig) -> Vec<String> {
    let before = serde_json::to_value(current.redacted()).expect("config serialization");
    let after = serde_json::to_value(target.redacted()).expect("config serialization");
    ["management_listen", "management_tls", "log_filter"]
        .into_iter()
        .filter(|field| before[*field] != after[*field])
        .map(str::to_owned)
        .collect()
}
fn changes(current: &DaemonConfig, target: &DaemonConfig) -> Vec<ConfigChange> {
    fn walk(
        path: String,
        before: &serde_json::Value,
        after: &serde_json::Value,
        out: &mut Vec<ConfigChange>,
    ) {
        if before == after {
            return;
        }
        if let (Some(before), Some(after)) = (before.as_object(), after.as_object()) {
            let keys: std::collections::BTreeSet<_> = before.keys().chain(after.keys()).collect();
            for key in keys {
                walk(
                    if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    },
                    before.get(key).unwrap_or(&serde_json::Value::Null),
                    after.get(key).unwrap_or(&serde_json::Value::Null),
                    out,
                );
            }
        } else {
            let restart = ["management_listen", "management_tls", "log_filter"]
                .iter()
                .any(|field| path == *field || path.starts_with(&format!("{field}.")));
            out.push(ConfigChange {
                path,
                before: before.clone(),
                after: after.clone(),
                apply_mode: if restart { "restart" } else { "live" }.into(),
            });
        }
    }
    let mut out = vec![];
    walk(
        String::new(),
        &serde_json::to_value(current.redacted()).unwrap(),
        &serde_json::to_value(target.redacted()).unwrap(),
        &mut out,
    );
    for upstream in &target.proxy.upstreams {
        let before = current
            .proxy
            .upstreams
            .iter()
            .find(|old| old.id == upstream.id)
            .and_then(|old| old.credentials.as_ref())
            .map(|credentials| (&credentials.username, &credentials.password));
        let after = upstream
            .credentials
            .as_ref()
            .map(|credentials| (&credentials.username, &credentials.password));
        if before != after {
            out.push(ConfigChange {
                path: format!("proxy.upstreams.{}.credentials", upstream.id),
                before: serde_json::Value::String("[REDACTED]".into()),
                after: serde_json::Value::String("[REDACTED: changed]".into()),
                apply_mode: "live".into(),
            });
        }
    }
    for provider in &target.providers {
        if let Some(old) = current.providers.iter().find(|old| old.id == provider.id)
            && hps_provider::fingerprint(old) != hps_provider::fingerprint(provider)
            && serde_json::to_value(old.redacted()).unwrap()
                == serde_json::to_value(provider.redacted()).unwrap()
        {
            out.push(ConfigChange {
                path: format!("providers.{}.credentials", provider.id),
                before: serde_json::json!("[REDACTED]"),
                after: serde_json::json!("[REDACTED: changed]"),
                apply_mode: "live".into(),
            });
        }
    }
    out
}
