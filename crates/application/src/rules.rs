use super::*;
use hps_config::{RoutingConfig, RuleDelete, RuleOrder, RuleToggle, RuleUpsert, RulesSave};
use hps_domain::{
    RouteAction, RouteSimulation, RuleCapabilities, RulesOverview, RulesSimulationInput,
};
use std::collections::{HashMap, HashSet};

fn only_routing_changed(
    current: &DaemonConfig,
    candidate: &DaemonConfig,
) -> Result<bool, ApplicationError> {
    let mut rest = candidate.clone();
    rest.proxy.routing = current.proxy.routing.clone();
    Ok(
        serde_json::to_value(current).map_err(|_| ApplicationError::Invalid)?
            == serde_json::to_value(rest).map_err(|_| ApplicationError::Invalid)?,
    )
}

impl Supervisor {
    async fn rule_base(
        &self,
        expected: i64,
        base: Option<i64>,
    ) -> Result<RoutingConfig, ApplicationError> {
        let current = self.current();
        let workspace = self.store.workspace().await?;
        if current.revision != expected || workspace.desired.revision.id != expected {
            return Err(StorageError::Conflict.into());
        }
        let Some(base) = base.filter(|&id| id != expected) else {
            return Ok(current.base_config.proxy.routing.clone());
        };
        if base <= 0 {
            return Err(ApplicationError::Invalid);
        }
        let document = self.store.revision(base).await?;
        if document.revision.parent_id != Some(expected) {
            return Err(StorageError::Conflict.into());
        }
        Ok(document.config.proxy.routing)
    }
    pub async fn rules_overview(
        &self,
        revision: Option<i64>,
    ) -> Result<RulesOverview<RoutingConfig>, ApplicationError> {
        let current = self.current();
        let workspace = self.store.workspace().await?;
        let viewed = revision.unwrap_or(current.revision);
        let document = self.store.revision(viewed).await?;
        let delta = changes(&current.base_config, &document.config);
        let other = !only_routing_changed(&current.base_config, &document.config)?;
        let capture = if let Some(capture) = &current.transparent {
            capture.status(current.revision).await
        } else {
            hps_domain::LinuxCaptureStatus::unconfigured(current.revision)
        };
        let local_process = capture.ready
            && capture
                .scope
                .as_ref()
                .is_some_and(|s| s.local_cgroup.is_some());
        let udp = capture.ready && capture.scope.as_ref().is_some_and(|s| s.udp);
        Ok(RulesOverview {
            running_revision: current.revision,
            desired_revision: workspace.desired.revision.id,
            viewed_revision: viewed,
            viewed_parent_revision: document.revision.parent_id,
            configuration: document.config.proxy.routing,
            active_configuration: current.base_config.proxy.routing.clone(),
            candidates: workspace
                .revisions
                .into_iter()
                .filter(|r| r.note == "Routing rules candidate" && r.applied_at.is_none())
                .take(50)
                .map(revision_view)
                .collect(),
            changes: delta,
            other_configuration_changes: other,
            capabilities: RuleCapabilities {
                local_process_available: local_process,
                local_process_reason: if local_process {
                    "Verified local cgroup capture is ready; explicit and gateway client processes remain unknown".into()
                } else {
                    capture.reason.unwrap_or_else(|| "No ready local cgroup capture; explicit and gateway client processes remain unknown".into())
                },
                udp_data_plane_available: udp,
                kernel_rule_revision: capture.kernel_revision,
                kernel_rules_ready: capture.ready,
            },
        })
    }
    pub async fn save_rules(
        self: &Arc<Self>,
        input: RulesSave,
    ) -> Result<ConfigRevision, ApplicationError> {
        self.rule_base(input.expected_revision, input.base_revision)
            .await?;
        let mut config = self.current().base_config.clone();
        config.proxy.routing = input.configuration;
        self.create_revision(ConfigInput {
            toml: toml::to_string(&config).map_err(|_| ApplicationError::Invalid)?,
            expected_revision: input.expected_revision,
            expected_draft_version: None,
            note: "Routing rules candidate".into(),
            use_draft_credentials: false,
        })
        .await
    }
    pub async fn upsert_rule(
        self: &Arc<Self>,
        input: RuleUpsert,
    ) -> Result<ConfigRevision, ApplicationError> {
        let mut configuration = self
            .rule_base(input.expected_revision, input.base_revision)
            .await?;
        let old = configuration
            .rules
            .iter()
            .position(|r| r.id == input.rule.id);
        if let Some(index) = old {
            configuration.rules.remove(index);
        }
        let index = input.index.or(old).unwrap_or(configuration.rules.len());
        if index > configuration.rules.len() {
            return Err(ApplicationError::Invalid);
        }
        configuration.rules.insert(index, input.rule);
        self.save_rules(RulesSave {
            configuration,
            expected_revision: input.expected_revision,
            base_revision: input.base_revision,
        })
        .await
    }
    pub async fn delete_rule(
        self: &Arc<Self>,
        input: RuleDelete,
    ) -> Result<ConfigRevision, ApplicationError> {
        let mut configuration = self
            .rule_base(input.expected_revision, input.base_revision)
            .await?;
        let count = configuration.rules.len();
        configuration.rules.retain(|r| r.id != input.id);
        if count == configuration.rules.len() {
            return Err(ApplicationError::RuleUnavailable);
        }
        self.save_rules(RulesSave {
            configuration,
            expected_revision: input.expected_revision,
            base_revision: input.base_revision,
        })
        .await
    }
    pub async fn toggle_rule(
        self: &Arc<Self>,
        input: RuleToggle,
    ) -> Result<ConfigRevision, ApplicationError> {
        let mut configuration = self
            .rule_base(input.expected_revision, input.base_revision)
            .await?;
        configuration
            .rules
            .iter_mut()
            .find(|r| r.id == input.id)
            .ok_or(ApplicationError::RuleUnavailable)?
            .enabled = input.enabled;
        self.save_rules(RulesSave {
            configuration,
            expected_revision: input.expected_revision,
            base_revision: input.base_revision,
        })
        .await
    }
    pub async fn reorder_rules(
        self: &Arc<Self>,
        input: RuleOrder,
    ) -> Result<ConfigRevision, ApplicationError> {
        let mut configuration = self
            .rule_base(input.expected_revision, input.base_revision)
            .await?;
        if input.ids.len() != configuration.rules.len()
            || input.ids.iter().collect::<HashSet<_>>().len() != input.ids.len()
        {
            return Err(ApplicationError::Invalid);
        }
        let mut rules = configuration
            .rules
            .drain(..)
            .map(|r| (r.id.clone(), r))
            .collect::<HashMap<_, _>>();
        configuration.rules = input
            .ids
            .into_iter()
            .map(|id| rules.remove(&id).ok_or(ApplicationError::Invalid))
            .collect::<Result<_, _>>()?;
        self.save_rules(RulesSave {
            configuration,
            expected_revision: input.expected_revision,
            base_revision: input.base_revision,
        })
        .await
    }
    pub async fn apply_rules(
        self: &Arc<Self>,
        input: ConfigActivate,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let current = self.current();
        if current.revision != input.expected_revision {
            return Err(StorageError::Conflict.into());
        }
        let document = self.store.revision(input.revision).await?;
        if !only_routing_changed(&current.base_config, &document.config)? {
            return Err(ApplicationError::RulesBase);
        }
        self.activate(input).await
    }
    pub async fn simulate_rules(
        self: &Arc<Self>,
        mut input: RulesSimulationInput,
    ) -> Result<RouteSimulation, ApplicationError> {
        let revision = input.revision.unwrap_or(self.current().revision);
        input.revision = Some(revision);
        let metadata = tasks::work_spec("rules_diagnostic", &input, Some(revision), None)?;
        self.owned_work(metadata,move|s|async move{s.simulate_rules_inner(input).await},|value|hps_storage::tasks::WorkResult {
            state:"completed",error_code:None,result_ref:Some(format!("config:{}",value.decision.configuration_revision)),
            result:Some(serde_json::json!({"configuration_revision":value.decision.configuration_revision,"rule_id":value.decision.rule_id,"action":value.decision.action,"action_supported":value.action_supported,"selected":value.selection.as_ref().and_then(|s|s.selected.as_ref())})),
        }).await
    }
    async fn simulate_rules_inner(
        &self,
        mut input: RulesSimulationInput,
    ) -> Result<RouteSimulation, ApplicationError> {
        let target = hps_engine::target::Target::new(
            &input.context.destination_host,
            input.context.destination_port,
        )
        .map_err(|_| ApplicationError::Invalid)?;
        let c = &input.context;
        if c.source_port == Some(0)
            || c.observed_domains.len() > 64
            || c.observed_domains
                .iter()
                .any(|d| !hps_config::valid_host(d) || d.parse::<std::net::IpAddr>().is_ok())
            || c.inbound.as_ref().is_some_and(|id| {
                id.is_empty()
                    || id.len() > 64
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
            || c.inbound_protocol
                .as_ref()
                .is_some_and(|p| !matches!(p.as_str(), "http" | "https" | "socks5" | "transparent"))
            || c.process.as_ref().is_some_and(|p| {
                p.pid == 0
                    || p.cgroup_id == 0
                    || p.command.is_empty()
                    || p.command.len() > 15
                    || p.command.chars().any(char::is_control)
            })
        {
            return Err(ApplicationError::Invalid);
        }
        input.context.destination_host = target.host;
        let permit = Arc::new(
            self.rule_diagnostics
                .clone()
                .try_acquire_owned()
                .map_err(|_| ApplicationError::RuleDiagnosticsCapacity)?,
        );
        let current = self.current();
        let revision = input.revision.unwrap_or(current.revision);
        let (engine, resolver) = if revision == current.revision {
            (current.engine.clone(), current.dns.clone())
        } else {
            let document = self.store.revision(revision).await?;
            let mut config = current.config.clone();
            config.proxy.routing = document.config.proxy.routing;
            let old = current.clone();
            let worker_permit = permit.clone();
            tokio::task::spawn_blocking(move || {
                let _permit = worker_permit;
                let resolver = dns::prepare(&config, Some(&old))?;
                let candidate = prepare_proxy_engine(&config, Some(&old.engine))?;
                let profiles = prepare_network(&config, &candidate, &resolver)?;
                let reuse = candidate.preparation_fingerprint()
                    == old.engine.preparation_fingerprint()
                    && resolver.fingerprint() == old.dns.fingerprint()
                    && profiles
                        .iter()
                        .map(|p| &p.fingerprint)
                        .eq(old.health_profiles.iter().map(|p| &p.fingerprint));
                Ok::<_, ApplicationError>((
                    if reuse { old.engine.clone() } else { candidate },
                    resolver,
                ))
            })
            .await
            .map_err(|_| ApplicationError::Worker)??
        };
        let declared = !input.context.observed_domains.is_empty();
        if !declared && let Ok(ip) = input.context.destination_host.parse() {
            input.context.observed_domains = resolver.domain_context(ip).domains;
        }
        let context = input.context.clone();
        let worker_permit = permit.clone();
        let mut result = tokio::task::spawn_blocking(move || {
            let _permit = worker_permit;
            engine.simulate_route(&context, revision)
        })
        .await
        .map_err(|_| ApplicationError::Worker)?;
        if revision != current.revision {
            result.notes.push("Candidate rules evaluated with current non-routing settings and live lease/health evidence".into());
        }
        if declared {
            result.notes.push(
                "DNS candidates were declared by the simulator caller, not observed for this flow"
                    .into(),
            );
        }
        if input.context.process.is_some() {
            result.notes.push("Declared process context is hypothetical; production process identity comes only from verified local kernel metadata".into());
        }
        Ok(result)
    }
    pub(super) fn validate_route_nodes(
        &self,
        candidate: &DaemonConfig,
        current: &RuntimeSnapshot,
    ) -> Result<(), ApplicationError> {
        if serde_json::to_value(&candidate.proxy.routing).map_err(|_| ApplicationError::Invalid)?
            == serde_json::to_value(&current.base_config.proxy.routing)
                .map_err(|_| ApplicationError::Invalid)?
        {
            return Ok(());
        }
        let mut required = HashSet::new();
        if let RouteAction::Node { id } = &candidate.proxy.routing.default_action
            && id.starts_with("lease:")
        {
            required.insert(id.clone());
        }
        for rule in candidate.proxy.routing.rules.iter().filter(|r| r.enabled) {
            if let RouteAction::Node { id } = &rule.action
                && id.starts_with("lease:")
            {
                required.insert(id.clone());
            }
        }
        if required.is_empty() {
            return Ok(());
        }
        let available = current
            .engine
            .probe_plans()
            .into_iter()
            .filter(|p| p.remaining().is_none_or(|r| !r.is_zero()))
            .map(|p| p.config().id.clone())
            .collect::<HashSet<_>>();
        if !required.is_subset(&available) {
            return Err(ApplicationError::RouteNodeUnavailable);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn diagnostic_capacity_rejects_before_work_and_recovers_without_production_demand() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        let service = Supervisor::boot(
            Store::open(None, config).unwrap(),
            StartupOverrides::default(),
        )
        .await
        .unwrap();
        let permits = (0..service.rule_diagnostics.available_permits())
            .map(|_| {
                service
                    .rule_diagnostics
                    .clone()
                    .try_acquire_owned()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let mut input = RulesSimulationInput {
            revision: None,
            context: hps_domain::RoutingContext {
                destination_host: "example.com".into(),
                destination_port: 443,
                transport: hps_domain::SelectionTransport::Tcp,
                source_ip: None,
                source_port: None,
                inbound: None,
                inbound_protocol: None,
                observed_domains: vec![],
                origin: hps_domain::FlowOrigin::Explicit,
                process: None,
            },
        };
        assert!(matches!(
            service.simulate_rules(input.clone()).await,
            Err(ApplicationError::RuleDiagnosticsCapacity)
        ));
        drop(permits);
        assert!(service.simulate_rules(input.clone()).await.is_ok());
        input.context.destination_host = "[2001:db8::1]".into();
        let ipv6 = service.simulate_rules(input).await.unwrap();
        assert!(ipv6.decision.domain.is_none());
        assert_eq!(ipv6.decision.domain_source, "unknown");
        assert!(service.current().engine.demand_age().is_none());
        service.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn expired_candidate_node_keeps_head_and_mixed_candidates_require_explicit_rebase() {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        let service = Supervisor::boot(
            Store::open(None, config).unwrap(),
            StartupOverrides::default(),
        )
        .await
        .unwrap();
        service.confirm_startup().await.unwrap();
        let current = service.current();
        let now = current.engine.dynamic_pool().logical_now();
        current
            .engine
            .set_provider_sources(HashMap::from([("fixture".into(), "v1".into())]));
        current
            .engine
            .dynamic_pool()
            .replace(vec![hps_engine::pool::PoolNode {
                provider_id: "fixture".into(),
                fingerprint: "v1".into(),
                min_remaining_secs: 0,
                upstream: hps_config::UpstreamConfig {
                    id: "lease:42".into(),
                    protocol: hps_config::ProxyProtocol::Http,
                    host: "127.0.0.1".into(),
                    port: 9,
                    enabled: true,
                    credentials: None,
                    expires_at_unix: Some(now + 60),
                    ca_certificate: None,
                    tls_server_name: None,
                },
            }])
            .unwrap();
        let candidate = service
            .save_rules(RulesSave {
                configuration: RoutingConfig {
                    default_action: RouteAction::Node {
                        id: "lease:42".into(),
                    },
                    ..Default::default()
                },
                expected_revision: 1,
                base_revision: None,
            })
            .await
            .unwrap();
        current.engine.dynamic_pool().replace(vec![]).unwrap();
        assert!(matches!(
            service
                .apply_rules(ConfigActivate {
                    revision: candidate.id,
                    expected_revision: 1,
                    defer_until_restart: false
                })
                .await,
            Err(ApplicationError::RouteNodeUnavailable)
        ));
        assert_eq!(service.current().revision, 1);
        assert_eq!(
            service.workspace().await.unwrap().activations[0]
                .error_code
                .as_deref(),
            Some("routing_node_unavailable")
        );
        let mut mixed = current.base_config.clone();
        mixed.instance_name = "different-instance-name".into();
        mixed.proxy.routing.default_action = RouteAction::Direct;
        let mixed = service
            .create_revision(ConfigInput {
                toml: toml::to_string(&mixed).unwrap(),
                expected_revision: 1,
                expected_draft_version: None,
                note: "mixed candidate".into(),
                use_draft_credentials: false,
            })
            .await
            .unwrap();
        assert!(
            service
                .rules_overview(Some(mixed.id))
                .await
                .unwrap()
                .other_configuration_changes
        );
        assert!(matches!(
            service
                .apply_rules(ConfigActivate {
                    revision: mixed.id,
                    expected_revision: 1,
                    defer_until_restart: false
                })
                .await,
            Err(ApplicationError::RulesBase)
        ));
        let rebased = service
            .save_rules(RulesSave {
                configuration: RoutingConfig {
                    default_action: RouteAction::Direct,
                    ..Default::default()
                },
                expected_revision: 1,
                base_revision: Some(mixed.id),
            })
            .await
            .unwrap();
        service
            .apply_rules(ConfigActivate {
                revision: rebased.id,
                expected_revision: 1,
                defer_until_restart: false,
            })
            .await
            .unwrap();
        assert_ne!(
            service.current().config.instance_name,
            "different-instance-name"
        );
        service.shutdown().await.unwrap();
    }
}
