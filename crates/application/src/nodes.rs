use super::*;
use hps_config::{GroupConfig, UpstreamConfig};
use hps_domain::{
    HealthVerdict, NodeActionInput, NodeActionResult, NodeOperationState, NodeQuery, NodeSummary,
    NodeView, NodesOverview,
};
use hps_storage::{nodes::LeaseScope, provider::NodeInventoryLease};
use std::collections::HashMap;

impl Supervisor {
    pub async fn nodes(
        self: &Arc<Self>,
        query: NodeQuery,
    ) -> Result<NodesOverview, ApplicationError> {
        validate_query(&query)?;
        let permit = self
            .node_operations
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::NodeCapacity)?;
        let service = self.clone();
        // This task owns the permit even when its HTTP/desktop caller goes away.
        self.provider_tasks
            .spawn(async move {
                let current = service.current();
                let leases = service.store.node_inventory_leases().await?;
                let tasks = service
                    .store
                    .latest_provider_tasks(
                        current
                            .config
                            .providers
                            .iter()
                            .map(|p| p.id.clone())
                            .collect(),
                    )
                    .await?;
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    inventory(&current, leases, query, tasks)
                })
                .await
                .map_err(|_| ApplicationError::Worker)?
            })
            .await
            .map_err(|_| ApplicationError::Worker)?
    }
    pub async fn node(self: &Arc<Self>, id: String) -> Result<NodesOverview, ApplicationError> {
        if !hps_config::valid_node_id(&id) {
            return Err(ApplicationError::Invalid);
        }
        // Search by stable ID then select exactly, so prefix matches never return a different node.
        let mut result = self
            .nodes(NodeQuery {
                id: Some(id.clone()),
                limit: Some(200),
                ..Default::default()
            })
            .await?;
        result.nodes.retain(|n| n.id == id);
        if result.nodes.is_empty() {
            return Err(StorageError::NotFound.into());
        }
        result.filtered_total = 1;
        result.next_offset = None;
        Ok(result)
    }
    pub async fn node_action(
        self: &Arc<Self>,
        input: NodeActionInput,
    ) -> Result<NodeActionResult, ApplicationError> {
        if !hps_config::valid_node_id(&input.id) || input.expected_control_version < 0 {
            return Err(ApplicationError::Invalid);
        }
        let state = match input.action.as_str() {
            "enable" => NodeOperationState::Enabled,
            "disable" => NodeOperationState::Disabled,
            "drain" => NodeOperationState::Draining,
            "delete" => NodeOperationState::Deleted,
            _ => return Err(ApplicationError::Invalid),
        };
        let permit = self
            .node_operations
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::NodeCapacity)?;
        let service = self.clone();
        self.provider_tasks
            .spawn(async move {
                let _permit = permit;
                let _operation = service.operation.lock().await;
                if service.closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(EngineError::ShuttingDown.into());
                }
                let current = service.current();
                if current.revision != input.expected_revision {
                    return Err(StorageError::Conflict.into());
                }
                let lease = if let Some(raw) = input.id.strip_prefix("lease:") {
                    let id = raw.parse::<i64>().map_err(|_| ApplicationError::Invalid)?;
                    let row = service.store.node_lease(id).await?;
                    if input.lease_id != Some(row.id)
                        || input.batch_id != Some(row.batch_id)
                        || input.provider_id.as_deref() != Some(row.provider_id.as_str())
                    {
                        return Err(StorageError::Conflict.into());
                    }
                    Some(LeaseScope {
                        id: row.id,
                        batch_id: row.batch_id,
                        provider_id: row.provider_id,
                    })
                } else {
                    if input.lease_id.is_some()
                        || input.batch_id.is_some()
                        || input.provider_id.is_some()
                    {
                        return Err(ApplicationError::Invalid);
                    }
                    if !current
                        .config
                        .proxy
                        .upstreams
                        .iter()
                        .any(|n| n.id == input.id)
                    {
                        return Err(StorageError::NotFound.into());
                    }
                    None
                };
                let control = service
                    .store
                    .set_node_control(
                        input.id,
                        state,
                        input.expected_control_version,
                        input.expected_revision,
                        lease,
                    )
                    .await?;
                let registry = current.engine.node_registry();
                registry.publish(control.clone());
                // Storage may have pruned controls whose lease history no longer exists.
                // The just-committed restriction is already applied if this diagnostic reload fails.
                if let Ok(controls) = service.store.node_controls().await {
                    registry.reconcile(controls);
                }
                let pool_refreshed = service.refresh_pool().await.is_ok();
                Ok(NodeActionResult {
                    pool_refreshed,
                    running_revision: current.revision,
                    control: Some(control),
                    configuration_activation: None,
                })
            })
            .await
            .map_err(|_| ApplicationError::Worker)?
    }
}

fn validate_query(q: &NodeQuery) -> Result<(), ApplicationError> {
    let valid_id = |s: &str| hps_config::valid_node_id(s) && !s.contains(':');
    if q.id.as_ref().is_some_and(|s| !hps_config::valid_node_id(s))
        || q.search.as_ref().is_some_and(|s| s.len() > 256)
        || q.provider
            .as_ref()
            .is_some_and(|s| s != "@static" && !valid_id(s))
        || q.group.as_ref().is_some_and(|s| !valid_id(s))
        || q.protocol
            .as_ref()
            .is_some_and(|s| !["http", "https", "socks5"].contains(&s.as_str()))
        || q.capability
            .as_ref()
            .is_some_and(|s| !["tcp", "udp", "ipv4", "ipv6"].contains(&s.as_str()))
        || q.state.as_ref().is_some_and(|s| {
            ![
                "eligible",
                "blocked",
                "expired",
                "near_expiry",
                "disabled",
                "draining",
                "deleted",
                "healthy",
                "unhealthy",
                "unknown",
                "retired",
            ]
            .contains(&s.as_str())
        })
        || q.sort
            .as_ref()
            .is_some_and(|s| !["id", "expiry_asc", "expiry_desc"].contains(&s.as_str()))
        || q.limit.is_some_and(|n| !(1..=200).contains(&n))
        || q.offset > 300000
    {
        return Err(ApplicationError::Invalid);
    }
    Ok(())
}
fn inventory(
    current: &RuntimeSnapshot,
    mut leases: Vec<NodeInventoryLease>,
    q: NodeQuery,
    tasks: Vec<hps_domain::ExtractionTask>,
) -> Result<NodesOverview, ApplicationError> {
    let truncated = leases.len() > 200000;
    leases.truncate(200000);
    let now = hps_storage::unix_now();
    let fingerprints = current
        .config
        .providers
        .iter()
        .map(|p| (p.id.as_str(), hps_provider::fingerprint(p)))
        .collect::<HashMap<_, _>>();
    let lease_now = current.engine.dynamic_pool().logical_now().max(now as u64);
    let plans = current
        .engine
        .probe_plans()
        .into_iter()
        .map(|p| (p.config().id.clone(), p))
        .collect::<HashMap<_, _>>();
    let mut rows = Vec::with_capacity(leases.len() + current.config.proxy.upstreams.len());
    let sources = current
        .config
        .proxy
        .upstreams
        .iter()
        .map(|n| (n, None))
        .chain(leases.iter().map(|l| (&l.upstream, Some(l))))
        .collect::<Vec<_>>();
    let mut summary = NodeSummary::default();
    let mut source_summaries = current
        .config
        .providers
        .iter()
        .map(|p| {
            let task = tasks.iter().find(|t| t.provider_id == p.id);
            hps_domain::ProviderNodeSummary {
                id: p.id.clone(),
                enabled: p.enabled,
                refill_enabled: p.refill.enabled,
                minimum: p.refill.min_nodes,
                target: p.refill.target_nodes,
                maximum: p.refill.max_nodes,
                inventory: 0,
                eligible: 0,
                near_expiry: 0,
                expired: 0,
                latest_task_id: task.map(|t| t.id),
                latest_task_state: task.map(|t| t.state.clone()),
                latest_error_code: task.and_then(|t| t.error_code.clone()),
                latest_task_at: task.map(|t| t.started_at),
            }
        })
        .collect::<Vec<_>>();
    let needle = q.search.as_ref().map(|s| s.to_lowercase());
    let requested_group = q.group.as_ref().and_then(|id| {
        current
            .config
            .proxy
            .selection
            .groups
            .iter()
            .find(|g| g.id == *id)
    });
    for (index, (up, lease)) in sources.iter().enumerate() {
        let lease = *lease;
        let plan = plans.get(&up.id);
        let node = make_view(current, up, lease, plan, now, lease_now, &fingerprints);
        summary.inventory += 1;
        summary.published += usize::from(node.published);
        summary.eligible += usize::from(node.eligible);
        summary.near_expiry += usize::from(node.remaining_secs.is_some_and(|n| n > 0 && n <= 60));
        summary.expired += usize::from(node.remaining_secs == Some(0));
        summary.restricted += usize::from(node.operation.state != NodeOperationState::Enabled);
        summary.source_unavailable += usize::from(node.lease_state.starts_with("source_"));
        if let Some(source) = source_summaries
            .iter_mut()
            .find(|s| node.provider_id.as_ref() == Some(&s.id))
        {
            source.inventory += 1;
            source.eligible += usize::from(node.eligible);
            source.near_expiry +=
                usize::from(node.remaining_secs.is_some_and(|n| n > 0 && n <= 60));
            source.expired += usize::from(node.remaining_secs == Some(0));
        }
        if q.id.as_ref().is_some_and(|id| *id != node.id)
            || needle.as_ref().is_some_and(|s| {
                !node.id.to_lowercase().contains(s) && !node.host.to_lowercase().contains(s)
            })
            || q.provider.as_ref().is_some_and(|p| {
                if p == "@static" {
                    node.provider_id.is_some()
                } else {
                    node.provider_id.as_ref() != Some(p)
                }
            })
            || q.group.is_some()
                && !requested_group.is_some_and(|g| group_matches(g, up, lease, &node.tags))
            || q.protocol.as_ref().is_some_and(|p| *p != node.protocol)
            || q.capability.as_ref().is_some_and(|c| match c.as_str() {
                "udp" => !node.udp_supported,
                "ipv4" | "ipv6" => !node.health_results.iter().any(|r| {
                    r.transport == hps_domain::HealthTransport::Tcp
                        && r.verdict == HealthVerdict::Healthy
                        && format!("{:?}", r.family).to_lowercase() == *c
                }),
                _ => !node.tcp_supported,
            })
            || q.remaining_max_secs
                .is_some_and(|max| node.remaining_secs.is_none_or(|n| n > max))
            || q.state.as_ref().is_some_and(|s| !state_matches(s, &node))
        {
            continue;
        }
        rows.push((index, node.id, node.remaining_secs));
    }
    summary.active_connections = current.engine.node_registry().total_active();
    rows.sort_by(|a, b| {
        let order = match q.sort.as_deref() {
            Some("expiry_asc") => a.2.unwrap_or(u64::MAX).cmp(&b.2.unwrap_or(u64::MAX)),
            Some("expiry_desc") => b.2.unwrap_or(u64::MAX).cmp(&a.2.unwrap_or(u64::MAX)),
            _ => std::cmp::Ordering::Equal,
        };
        order.then_with(|| a.1.cmp(&b.1))
    });
    let total = rows.len();
    let offset = q.offset as usize;
    let limit = q.limit.unwrap_or(50) as usize;
    let nodes = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(index, _, _)| {
            let (up, lease) = sources[index];
            let mut node = make_view(
                current,
                up,
                lease,
                plans.get(&up.id),
                now,
                lease_now,
                &fingerprints,
            );
            node.groups = current
                .config
                .proxy
                .selection
                .groups
                .iter()
                .filter(|g| group_matches(g, up, lease, &node.tags))
                .map(|g| g.id.clone())
                .collect();
            node
        })
        .collect();
    Ok(NodesOverview {
        running_revision: current.revision,
        evaluated_at_unix: now,
        summary,
        inventory_truncated: truncated,
        providers: current
            .config
            .providers
            .iter()
            .map(|p| p.id.clone())
            .collect(),
        groups: current
            .config
            .proxy
            .selection
            .groups
            .iter()
            .map(|g| g.id.clone())
            .collect(),
        sources: source_summaries,
        filtered_total: total,
        offset: q.offset,
        next_offset: (offset + limit < total).then_some((offset + limit) as u32),
        nodes,
    })
}
fn make_view(
    current: &RuntimeSnapshot,
    up: &UpstreamConfig,
    lease: Option<&NodeInventoryLease>,
    plan: Option<&hps_engine::dial::ProbePlan>,
    now: i64,
    lease_now: u64,
    fingerprints: &HashMap<&str, String>,
) -> NodeView {
    let registry = current.engine.node_registry();
    let operation = registry.control(&up.id);
    let source = lease.and_then(|l| {
        current
            .config
            .providers
            .iter()
            .find(|p| p.id == l.provider_id)
    });
    let remaining_duration = plan.and_then(|p| p.remaining()).or_else(|| {
        up.expires_at_unix.map(|expiry| {
            std::time::Duration::from_secs(expiry.saturating_sub(if lease.is_some() {
                lease_now
            } else {
                now as u64
            }))
        })
    });
    let remaining =
        remaining_duration.map(|r| r.as_secs().saturating_add(u64::from(r.subsec_nanos() > 0)));
    let lease_state = match lease {
        Some(l) if l.state != "active" => l.state.as_str(),
        _ if remaining == Some(0) => "expired",
        Some(_) if source.is_none() => "source_deleted",
        Some(_) if source.is_some_and(|p| !p.enabled) => "source_disabled",
        Some(l)
            if fingerprints
                .get(l.provider_id.as_str())
                .is_some_and(|fp| **fp != l.fingerprint) =>
        {
            "source_changed"
        }
        Some(_) => "active",
        None => "static",
    }
    .to_owned();
    let mut reasons = vec![];
    if !up.enabled {
        reasons.push("configuration_disabled".into());
    }
    if let Some(reason) = operation.state.blocking_reason() {
        reasons.push(reason.into());
    }
    if !["active", "static"].contains(&lease_state.as_str()) {
        reasons.push(lease_state.clone());
    }
    let minimum = current
        .config
        .proxy
        .limits
        .min_remaining_lifetime_secs
        .max(source.map_or(0, |p| {
            if p.refill.enabled {
                p.min_remaining_secs.max(p.refill.retire_before_secs)
            } else {
                p.min_remaining_secs
            }
        }));
    if remaining_duration
        .is_some_and(|r| !r.is_zero() && r <= std::time::Duration::from_secs(minimum))
    {
        reasons.push("insufficient_lifetime".into());
    }
    let health_results = plan
        .map(|p| {
            current
                .health_profiles
                .iter()
                .filter_map(|profile| {
                    current.engine.health_registry().latest(
                        p.identity(),
                        &profile.fingerprint,
                        current.config.health.max_staleness_secs,
                    )
                })
                .map(|mut result| {
                    result.node_id = up.id.clone();
                    result
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let healthy = plan.is_some_and(|p| current.engine.health_usable(p.identity()));
    let health_state = if !current.config.health.enabled {
        "checking_disabled"
    } else if healthy {
        "healthy"
    } else if health_results
        .iter()
        .any(|r| r.verdict == HealthVerdict::Failed)
    {
        "unhealthy"
    } else {
        "unknown"
    }
    .to_owned();
    if current.config.health.enabled && !healthy {
        reasons.push("health_unavailable".into());
    }
    if plan.is_none() {
        reasons.push("not_published".into());
    }
    let mut tags = current
        .config
        .proxy
        .selection
        .node_tags
        .get(&up.id)
        .cloned()
        .unwrap_or_default();
    if let Some(extra) = lease.and_then(|l| {
        current
            .config
            .proxy
            .selection
            .provider_tags
            .get(&l.provider_id)
    }) {
        tags.extend(extra.iter().cloned());
    }
    tags.sort();
    tags.dedup();
    NodeView {
        id: up.id.clone(),
        kind: if lease.is_some() { "lease" } else { "static" }.into(),
        provider_id: lease.map(|l| l.provider_id.clone()),
        lease_id: lease.map(|l| l.id),
        batch_id: lease.map(|l| l.batch_id),
        protocol: format!("{:?}", up.protocol).to_lowercase(),
        host: up.host.clone(),
        port: up.port,
        configuration_enabled: up.enabled,
        has_credentials: lease.map_or(up.credentials.is_some(), |l| l.has_credentials),
        received_at: lease.map(|l| l.received_at),
        expires_at_unix: up.expires_at_unix,
        remaining_secs: remaining,
        expiry_basis: lease.map(|l| l.expiry_basis.clone()),
        region: lease.and_then(|l| l.region.clone()),
        lease_state,
        operation,
        published: plan.is_some(),
        eligible: reasons.is_empty(),
        admission_reasons: reasons,
        tcp_supported: true,
        udp_supported: false,
        health_state,
        health_results,
        groups: vec![],
        tags,
        active_connections: registry.active(&up.id),
    }
}
fn group_matches(
    g: &GroupConfig,
    up: &UpstreamConfig,
    lease: Option<&NodeInventoryLease>,
    tags: &[String],
) -> bool {
    (g.include_all
        || g.members.contains(&up.id)
        || lease.is_some_and(|l| g.providers.contains(&l.provider_id)))
        && g.tags.iter().all(|t| tags.contains(t))
        && (g.protocols.is_empty() || g.protocols.contains(&up.protocol))
}
fn state_matches(state: &str, n: &NodeView) -> bool {
    match state {
        "eligible" => n.eligible,
        "blocked" => !n.eligible,
        "expired" => n.remaining_secs == Some(0),
        "near_expiry" => n.remaining_secs.is_some_and(|r| r > 0 && r <= 60),
        "disabled" => !n.configuration_enabled || n.operation.state == NodeOperationState::Disabled,
        "draining" => n.operation.state == NodeOperationState::Draining,
        "deleted" => n.operation.state == NodeOperationState::Deleted,
        "retired" => !["active", "static"].contains(&n.lease_state.as_str()),
        s => n.health_state == s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> Arc<Supervisor> {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        for (id, enabled, expires) in [
            ("manual", true, None),
            ("disabled", false, None),
            ("expired", true, Some(1)),
        ] {
            config.proxy.upstreams.push(UpstreamConfig {
                id: id.into(),
                protocol: hps_config::ProxyProtocol::Http,
                host: "192.0.2.8".into(),
                port: 8080,
                enabled,
                expires_at_unix: expires,
                credentials: Some(hps_config::UpstreamCredentials {
                    username: "do-not-return".into(),
                    password: "private-credential".into(),
                }),
                ca_certificate: None,
                tls_server_name: None,
            });
        }
        config.proxy.selection.groups.push(GroupConfig {
            id: "static-only".into(),
            members: vec!["manual".into()],
            ..Default::default()
        });
        for id in ["vendor-a", "vendor-b"] {
            config.providers.push(hps_config::ProviderConfig {
                id: id.into(),
                enabled: true,
                kind: hps_config::ProviderKind::Text,
                endpoint: "http://localhost/list".into(),
                ttl_secs: Some(300),
                ..Default::default()
            });
        }
        let store = Store::open(None, config.clone()).unwrap();
        for provider in config.providers {
            let (task, _) = store
                .begin_extraction(
                    provider.clone(),
                    "first".into(),
                    hps_provider::fingerprint(&provider),
                )
                .await
                .unwrap();
            store.extraction_requesting(task.id).await.unwrap();
            let nodes =
                hps_provider::parse(&provider, b"192.0.2.1:8080", hps_storage::unix_now()).unwrap();
            store
                .finish_extraction(task.id, provider, nodes, true)
                .await
                .unwrap();
        }
        let service = Supervisor::boot(store, StartupOverrides::default())
            .await
            .unwrap();
        service.confirm_startup().await.unwrap();
        service
    }
    fn action(n: &NodeView, revision: i64, kind: &str) -> NodeActionInput {
        NodeActionInput {
            id: n.id.clone(),
            action: kind.into(),
            expected_revision: revision,
            expected_control_version: n.operation.version,
            lease_id: n.lease_id,
            batch_id: n.batch_id,
            provider_id: n.provider_id.clone(),
        }
    }
    #[tokio::test]
    async fn inventory_separates_expiry_health_operational_controls_and_exact_lease_ownership() {
        let service = fixture().await;
        let all = service.nodes(NodeQuery::default()).await.unwrap();
        assert_eq!(all.summary.inventory, 5);
        assert_eq!(all.summary.eligible, 3);
        assert_eq!(all.summary.expired, 1);
        assert_eq!(all.sources.len(), 2);
        assert!(
            all.sources
                .iter()
                .all(|s| s.latest_task_state.as_deref() == Some("succeeded") && s.eligible == 1)
        );
        let json = serde_json::to_string(&all).unwrap();
        assert!(!json.contains("private-credential"));
        assert!(!json.contains("do-not-return"));
        let expired = all.nodes.iter().find(|n| n.id == "expired").unwrap();
        assert_eq!(expired.lease_state, "expired");
        assert_eq!(expired.health_state, "checking_disabled");
        let page = service
            .nodes(NodeQuery {
                limit: Some(2),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.next_offset, Some(2));
        let second = service
            .nodes(NodeQuery {
                limit: Some(2),
                offset: 2,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_ne!(page.nodes[0].id, second.nodes[0].id);
        let group = service
            .nodes(NodeQuery {
                group: Some("static-only".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(group.nodes.len(), 1);
        assert_eq!(group.nodes[0].id, "manual");
        let lease = all
            .nodes
            .iter()
            .find(|n| n.provider_id.as_deref() == Some("vendor-a"))
            .unwrap();
        let mut wrong = action(lease, 1, "delete");
        wrong.batch_id = Some(999);
        assert!(matches!(
            service.node_action(wrong).await,
            Err(ApplicationError::Storage(StorageError::Conflict))
        ));
        service
            .node_action(action(lease, 1, "delete"))
            .await
            .unwrap();
        assert!(matches!(
            service.node_action(action(lease, 1, "enable")).await,
            Err(ApplicationError::Storage(StorageError::Conflict))
        ));
        let selected = service.node(lease.id.clone()).await.unwrap();
        let selected = &selected.nodes[0];
        assert!(!selected.eligible);
        assert_eq!(selected.operation.state, NodeOperationState::Deleted);
        assert_eq!(selected.expires_at_unix, lease.expires_at_unix);
        assert_eq!(selected.lease_state, "active");
        let other = service
            .nodes(NodeQuery {
                provider: Some("vendor-b".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(other.nodes[0].eligible);
        assert_eq!(
            service
                .provider_controller("vendor-a")
                .await
                .unwrap()
                .current_inventory,
            0
        );
        assert_eq!(
            service
                .provider_controller("vendor-b")
                .await
                .unwrap()
                .current_inventory,
            1
        );
        assert!(
            service
                .nodes(NodeQuery {
                    capability: Some("udp".into()),
                    ..Default::default()
                })
                .await
                .unwrap()
                .nodes
                .is_empty()
        );
        let disabled = all.nodes.iter().find(|n| n.id == "disabled").unwrap();
        service
            .node_action(action(disabled, 1, "enable"))
            .await
            .unwrap();
        assert!(!service.node("disabled".into()).await.unwrap().nodes[0].eligible);
        service.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn caller_cancellation_does_not_abandon_accepted_node_action() {
        let service = fixture().await;
        let node = service.node("manual".into()).await.unwrap().nodes.remove(0);
        let lock = service.operation.lock().await;
        let worker = service.clone();
        let caller =
            tokio::spawn(async move { worker.node_action(action(&node, 1, "drain")).await });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while service.node_operations.available_permits() == 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        caller.abort();
        drop(lock);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while service
                .current()
                .engine
                .node_registry()
                .control("manual")
                .version
                == 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            service
                .current()
                .engine
                .node_registry()
                .control("manual")
                .state,
            NodeOperationState::Draining
        );
        service.shutdown().await.unwrap();
    }
}
