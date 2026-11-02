use super::*;
use hps_domain::{
    ConnectionDisconnectConfirm, ConnectionDisconnectInput, ConnectionDisconnectPreview,
    ConnectionDisconnectPreviewInput, ConnectionDisconnectResult, ConnectionDisconnectTarget,
    ConnectionInfo, ConnectionQuery, ConnectionStatistics, ConnectionsOverview,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const TTL: Duration = Duration::from_secs(30);
const MAX_SNAPSHOTS: usize = 4;
const MAX_PREVIEWS: usize = 8;
const MAX_DISCONNECT: usize = 500;
fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
struct Snapshot {
    epoch: String,
    fingerprint: String,
    deadline: Instant,
    created: u64,
    expires: u64,
    revision: i64,
    statistics: ConnectionStatistics,
    records: Vec<ConnectionInfo>,
    inbounds: Vec<String>,
    groups: Vec<String>,
    providers: Vec<String>,
}
struct Preview {
    deadline: Instant,
    view: ConnectionDisconnectPreview,
    result: Option<ConnectionDisconnectResult>,
}
pub(super) struct ConnectionRuntime {
    slots: Arc<tokio::sync::Semaphore>,
    snapshots: Mutex<HashMap<String, Arc<Snapshot>>>,
    previews: Mutex<HashMap<String, Preview>>,
}
impl Default for ConnectionRuntime {
    fn default() -> Self {
        Self {
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
            snapshots: Mutex::new(HashMap::new()),
            previews: Mutex::new(HashMap::new()),
        }
    }
}
impl Supervisor {
    pub async fn connection_summary(
        self: &Arc<Self>,
    ) -> Result<hps_domain::ConnectionSummary, ApplicationError> {
        let permit = self
            .observations
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::ConnectionCapacity)?;
        let operation = self.operation.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        let service = self.clone();
        let task = self.provider_tasks.spawn(async move {
            let current = service.current();
            let (activation, tasks) = tokio::try_join!(
                service.store.latest_activation(),
                service.store.latest_provider_tasks(
                    current
                        .config
                        .providers
                        .iter()
                        .filter(|p| p.enabled)
                        .map(|p| p.id.clone())
                        .collect()
                )
            )?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let now = wall_ms();
                let proxy = current.engine.status();
                let mut attention = vec![];
                if !proxy.inbounds.iter().any(|i| i.running) {
                    attention.push(hps_domain::ConnectionAttention {
                        kind: "inbound".into(),
                        code: "no_running_inbound".into(),
                        severity: "info".into(),
                        resource_id: None,
                        observed_at_unix_ms: now,
                        href: "./inbounds.html".into(),
                    });
                } else if !current.engine.ready() {
                    attention.push(hps_domain::ConnectionAttention {
                        kind: "egress".into(),
                        code: "no_forwarding_path".into(),
                        severity: "warning".into(),
                        resource_id: None,
                        observed_at_unix_ms: now,
                        href: "./rules.html".into(),
                    });
                }
                if let Some((revision, _)) = service.pending_restart.lock().unwrap().as_ref() {
                    attention.push(hps_domain::ConnectionAttention {
                        kind: "configuration".into(),
                        code: "pending_restart".into(),
                        severity: "info".into(),
                        resource_id: Some(revision.to_string()),
                        observed_at_unix_ms: now,
                        href: "./config.html".into(),
                    });
                }
                if let Some(activation) = activation
                    && activation.state == "failed"
                {
                    let code = activation
                        .error_code
                        .unwrap_or_else(|| "configuration_activation_failed".into());
                    attention.push(hps_domain::ConnectionAttention {
                        kind: if code == "dns_capture_preparation_failed" {
                            "linux_dns_capture"
                        } else {
                            "configuration"
                        }
                        .into(),
                        code,
                        severity: "warning".into(),
                        resource_id: Some(activation.revision.to_string()),
                        observed_at_unix_ms: (activation.created_at.max(0) as u64)
                            .saturating_mul(1000),
                        href: "./config.html".into(),
                    });
                }
                for task in tasks {
                    if matches!(task.state.as_str(), "failed" | "outcome_unknown")
                        && current.config.providers.iter().any(|p| {
                            p.id == task.provider_id
                                && hps_provider::fingerprint(p) == task.config_fingerprint
                        })
                    {
                        attention.push(hps_domain::ConnectionAttention {
                            kind: "provider".into(),
                            code: task
                                .error_code
                                .unwrap_or_else(|| "extraction_failed".into()),
                            severity: "warning".into(),
                            resource_id: Some(task.provider_id.clone()),
                            observed_at_unix_ms:
                                (task.finished_at.unwrap_or(task.started_at).max(0) as u64)
                                    .saturating_mul(1000),
                            href: format!("./provider-detail.html?id={}", task.provider_id),
                        });
                    }
                }
                if current
                    .capture
                    .as_ref()
                    .is_some_and(|capture| capture.status().map_or(true, |status| !status.attached))
                {
                    attention.push(hps_domain::ConnectionAttention {
                        kind: "linux_dns_capture".into(),
                        code: "dns_capture_not_attached".into(),
                        severity: "warning".into(),
                        resource_id: None,
                        observed_at_unix_ms: now,
                        href: "./dns.html".into(),
                    });
                }
                attention.truncate(32);
                Ok(hps_domain::ConnectionSummary {
                    running_revision: current.revision,
                    evaluated_at_unix_ms: now,
                    statistics: current.engine.node_registry().connection_statistics(),
                    proxy,
                    attention,
                })
            })
            .await
            .map_err(|_| ApplicationError::Worker)?
        });
        drop(operation);
        task.await.map_err(|_| ApplicationError::Worker)?
    }
    async fn connection_work<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(Arc<Self>) -> Result<T, ApplicationError> + Send + 'static,
    ) -> Result<T, ApplicationError> {
        let permit = self
            .observations
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::ConnectionCapacity)?;
        // Register ownership before shutdown can close/wait on the task tracker.
        let operation = self.operation.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        let service = self.clone();
        let task = self.provider_tasks.spawn(async move {
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                work(service)
            })
            .await
            .map_err(|_| ApplicationError::Worker)?
        });
        drop(operation);
        task.await.map_err(|_| ApplicationError::Worker)?
    }
    pub async fn connections(
        self: &Arc<Self>,
        query: ConnectionQuery,
    ) -> Result<ConnectionsOverview, ApplicationError> {
        validate(&query)?;
        self.connection_work(move |service| {
            let current = service.current();
            let registry = current.engine.node_registry();
            check_epoch(
                query.expected_epoch.as_deref(),
                registry.observation_epoch(),
            )?;
            let fingerprint = fingerprint(&query);
            let (id, snapshot) = if let Some(id) = &query.snapshot {
                let snapshot = service.observations.snapshot(id)?;
                check_epoch(Some(&snapshot.epoch), registry.observation_epoch())?;
                if fingerprint != snapshot.fingerprint {
                    return Err(ApplicationError::ConnectionFilters);
                }
                (id.clone(), snapshot)
            } else {
                let (statistics, mut records) = registry.connection_records(true);
                let source = source_filter(query.source.as_deref())?;
                records.retain(|record| matches_query(record, &query, source.as_ref()));
                records.sort_by(|a, b| {
                    let order = match query.sort.as_deref() {
                        Some("oldest") => a.id.cmp(&b.id),
                        Some("bytes_up") => b.bytes_up.cmp(&a.bytes_up),
                        Some("bytes_down") => b.bytes_down.cmp(&a.bytes_down),
                        Some("elapsed") => b.elapsed_ms.cmp(&a.elapsed_ms),
                        _ => b.id.cmp(&a.id),
                    };
                    order.then_with(|| b.id.cmp(&a.id))
                });
                let created = wall_ms();
                let id = uuid::Uuid::new_v4().to_string();
                let snapshot = Arc::new(Snapshot {
                    epoch: statistics.epoch.clone(),
                    fingerprint,
                    deadline: Instant::now() + TTL,
                    created,
                    expires: created.saturating_add(TTL.as_millis() as u64),
                    revision: current.revision,
                    statistics,
                    records,
                    inbounds: current
                        .config
                        .proxy
                        .inbounds
                        .iter()
                        .map(|i| i.id.clone())
                        .collect(),
                    groups: current
                        .config
                        .proxy
                        .selection
                        .groups
                        .iter()
                        .map(|g| g.id.clone())
                        .collect(),
                    providers: current
                        .config
                        .providers
                        .iter()
                        .map(|p| p.id.clone())
                        .chain(current.config.subscriptions.iter().map(|s| s.source_id()))
                        .collect(),
                });
                let mut snapshots = service.observations.snapshots.lock().unwrap();
                snapshots.retain(|_, s| s.deadline > Instant::now());
                if snapshots.len() >= MAX_SNAPSHOTS
                    && let Some(oldest) = snapshots
                        .iter()
                        .min_by_key(|(_, s)| s.deadline)
                        .map(|(id, _)| id.clone())
                {
                    snapshots.remove(&oldest);
                }
                snapshots.insert(id.clone(), snapshot.clone());
                (id, snapshot)
            };
            let offset = query.offset as usize;
            let limit = query.limit.unwrap_or(50) as usize;
            let total = snapshot.records.len();
            Ok(ConnectionsOverview {
                running_revision: snapshot.revision,
                epoch: snapshot.epoch.clone(),
                snapshot: id,
                snapshot_created_at_unix_ms: snapshot.created,
                snapshot_expires_at_unix_ms: snapshot.expires,
                statistics: snapshot.statistics.clone(),
                filtered_total: total,
                offset: query.offset,
                next_offset: (offset + limit < total).then_some((offset + limit) as u32),
                records: snapshot
                    .records
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .cloned()
                    .collect(),
                inbounds: snapshot.inbounds.clone(),
                groups: snapshot.groups.clone(),
                providers: snapshot.providers.clone(),
            })
        })
        .await
    }
    pub async fn connection(
        self: &Arc<Self>,
        id: String,
    ) -> Result<ConnectionInfo, ApplicationError> {
        if !hps_domain::valid_connection_id(&id) {
            return Err(ApplicationError::ConnectionInput);
        }
        self.connection_work(move |service| {
            let registry = service.current().engine.node_registry();
            check_epoch(
                id.split_once(':').map(|(e, _)| e),
                registry.observation_epoch(),
            )?;
            registry
                .connection_record(&id)
                .ok_or(ApplicationError::ConnectionUnavailable)
        })
        .await
    }
    pub async fn disconnect_connection(
        self: &Arc<Self>,
        input: ConnectionDisconnectInput,
    ) -> Result<ConnectionDisconnectResult, ApplicationError> {
        if !hps_domain::valid_connection_id(&input.id) {
            return Err(ApplicationError::ConnectionInput);
        }
        self.connection_work(move |service| {
            let registry = service.current().engine.node_registry();
            check_epoch(Some(&input.expected_epoch), registry.observation_epoch())?;
            check_epoch(
                input.id.split_once(':').map(|(e, _)| e),
                registry.observation_epoch(),
            )?;
            let requested = usize::from(registry.disconnect_connection(&input.id));
            Ok(ConnectionDisconnectResult {
                epoch: registry.observation_epoch().into(),
                token: None,
                requested,
                already_ended: 1 - requested,
                matched: 1,
                requested_at_unix_ms: wall_ms(),
            })
        })
        .await
    }
    pub async fn preview_disconnect_connections(
        self: &Arc<Self>,
        input: ConnectionDisconnectPreviewInput,
    ) -> Result<ConnectionDisconnectPreview, ApplicationError> {
        if input.ids.len() > MAX_DISCONNECT
            || input
                .ids
                .iter()
                .any(|id| !hps_domain::valid_connection_id(id))
        {
            return Err(ApplicationError::ConnectionInput);
        }
        self.connection_work(move |service| {
            let registry = service.current().engine.node_registry();
            check_epoch(Some(&input.expected_epoch), registry.observation_epoch())?;
            let snapshot = service.observations.snapshot(&input.snapshot)?;
            check_epoch(Some(&snapshot.epoch), registry.observation_epoch())?;
            let ids = input.ids.iter().collect::<HashSet<_>>();
            if ids.len() != input.ids.len()
                || ids
                    .iter()
                    .any(|id| !snapshot.records.iter().any(|r| &r.id == *id))
            {
                return Err(ApplicationError::ConnectionInput);
            }
            let records = snapshot
                .records
                .iter()
                .filter(|r| r.ended_at_unix_ms.is_none() && (ids.is_empty() || ids.contains(&r.id)))
                .collect::<Vec<_>>();
            if records.len() > MAX_DISCONNECT {
                return Err(ApplicationError::ConnectionScope);
            }
            let targets = records
                .into_iter()
                .map(|r| ConnectionDisconnectTarget {
                    id: r.id.clone(),
                    source_ip: r.context.source_ip,
                    source_port: r.context.source_port,
                    inbound: r.context.inbound.clone(),
                    destination_host: r.context.destination_host.clone(),
                    destination_port: r.context.destination_port,
                    node_id: r.choice.as_ref().map(|c| c.node_id.clone()),
                    configuration_revision: r.routing.configuration_revision,
                })
                .collect();
            let created = wall_ms();
            let token = uuid::Uuid::new_v4().to_string();
            let view = ConnectionDisconnectPreview {
                token: token.clone(),
                epoch: snapshot.epoch.clone(),
                created_at_unix_ms: created,
                expires_at_unix_ms: created.saturating_add(TTL.as_millis() as u64),
                targets,
                active_untracked_in_instance: snapshot.statistics.active_untracked,
            };
            let mut previews = service.observations.previews.lock().unwrap();
            previews.retain(|_, p| p.deadline > Instant::now());
            if previews.len() >= MAX_PREVIEWS
                && let Some(oldest) = previews
                    .iter()
                    .min_by_key(|(_, p)| p.deadline)
                    .map(|(id, _)| id.clone())
            {
                previews.remove(&oldest);
            }
            previews.insert(
                token,
                Preview {
                    deadline: Instant::now() + TTL,
                    view: view.clone(),
                    result: None,
                },
            );
            Ok(view)
        })
        .await
    }
    pub async fn confirm_disconnect_connections(
        self: &Arc<Self>,
        input: ConnectionDisconnectConfirm,
    ) -> Result<ConnectionDisconnectResult, ApplicationError> {
        if uuid::Uuid::parse_str(&input.token).is_err() {
            return Err(ApplicationError::ConnectionInput);
        }
        self.connection_work(move |service| {
            let registry = service.current().engine.node_registry();
            check_epoch(Some(&input.expected_epoch), registry.observation_epoch())?;
            // Hold only this bounded in-memory preview lock through cancellation, so retries
            // return the same accepted result and never recalculate a scope from live filters.
            let mut previews = service.observations.previews.lock().unwrap();
            previews.retain(|_, p| p.deadline > Instant::now());
            let preview = previews
                .get_mut(&input.token)
                .ok_or(ApplicationError::ConnectionSnapshot)?;
            check_epoch(Some(&preview.view.epoch), registry.observation_epoch())?;
            if let Some(result) = &preview.result {
                return Ok(result.clone());
            }
            let requested = preview
                .view
                .targets
                .iter()
                .filter(|target| registry.disconnect_connection(&target.id))
                .count();
            let result = ConnectionDisconnectResult {
                epoch: preview.view.epoch.clone(),
                token: Some(input.token),
                requested,
                already_ended: preview.view.targets.len() - requested,
                matched: preview.view.targets.len(),
                requested_at_unix_ms: wall_ms(),
            };
            preview.result = Some(result.clone());
            Ok(result)
        })
        .await
    }
}
impl ConnectionRuntime {
    fn snapshot(&self, id: &str) -> Result<Arc<Snapshot>, ApplicationError> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(ApplicationError::ConnectionInput);
        }
        let mut snapshots = self.snapshots.lock().unwrap();
        snapshots.retain(|_, s| s.deadline > Instant::now());
        snapshots
            .get(id)
            .cloned()
            .ok_or(ApplicationError::ConnectionSnapshot)
    }
}
fn check_epoch(expected: Option<&str>, actual: &str) -> Result<(), ApplicationError> {
    if expected.is_some_and(|e| !hps_domain::valid_connection_epoch(e)) {
        return Err(ApplicationError::ConnectionInput);
    }
    if expected.is_some_and(|e| e != actual) {
        return Err(ApplicationError::ConnectionEpoch);
    }
    Ok(())
}
fn fingerprint(query: &ConnectionQuery) -> String {
    let mut q = query.clone();
    q.snapshot = None;
    q.expected_epoch = None;
    q.offset = 0;
    q.limit = None;
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&q).expect("connection filter serialization"))
    )
}
fn valid_resource(id: &str) -> bool {
    hps_config::valid_node_id(id) && !id.contains(':')
}
fn validate(q: &ConnectionQuery) -> Result<(), ApplicationError> {
    if q.limit.is_some_and(|l| !(1..=200).contains(&l))
        || q.offset > 10512
        || q.id
            .as_ref()
            .is_some_and(|s| !hps_domain::valid_connection_id(s))
        || q.expected_epoch
            .as_ref()
            .is_some_and(|e| !hps_domain::valid_connection_epoch(e))
        || q.snapshot
            .as_ref()
            .is_some_and(|s| uuid::Uuid::parse_str(s).is_err())
        || q.source_port == Some(0)
        || q.destination_port == Some(0)
        || q.destination.as_ref().is_some_and(|s| {
            s.is_empty() || s.len() > 256 || !s.is_ascii() || s.chars().any(char::is_control)
        })
        || q.inbound
            .as_ref()
            .is_some_and(|s| s != "@unknown" && !valid_resource(s))
        || q.node
            .as_ref()
            .is_some_and(|s| s != "@direct" && !hps_config::valid_node_id(s))
        || q.provider
            .as_ref()
            .is_some_and(|s| s != "@static" && !hps_config::valid_source_id(s))
        || q.group
            .as_ref()
            .is_some_and(|s| s != "@none" && !valid_resource(s))
        || q.rule
            .as_ref()
            .is_some_and(|s| s != "@default" && !valid_resource(s))
        || q.protocol
            .as_ref()
            .is_some_and(|s| !["http", "https", "socks5", "transparent"].contains(&s.as_str()))
        || q.transport
            .as_ref()
            .is_some_and(|s| !["tcp", "udp"].contains(&s.as_str()))
        || q.origin
            .as_ref()
            .is_some_and(|s| !["explicit", "linux_local", "gateway"].contains(&s.as_str()))
        || q.state.as_ref().is_some_and(|s| {
            ![
                "active",
                "all",
                "ended",
                "open",
                "dialing",
                "failed",
                "rejected",
                "disconnected",
                "closed",
                "disconnecting",
            ]
            .contains(&s.as_str())
        })
        || q.sort.as_ref().is_some_and(|s| {
            !["newest", "oldest", "bytes_up", "bytes_down", "elapsed"].contains(&s.as_str())
        })
        || q.error.as_ref().is_some_and(|s| {
            s.is_empty() || s.len() > 64 || !s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        })
    {
        return Err(ApplicationError::ConnectionInput);
    }
    source_filter(q.source.as_deref())?;
    Ok(())
}
fn source_filter(source: Option<&str>) -> Result<Option<ipnet::IpNet>, ApplicationError> {
    source
        .map(|s| {
            if s.len() > 64 {
                return Err(ApplicationError::ConnectionInput);
            }
            s.parse::<ipnet::IpNet>()
                .or_else(|_| {
                    s.parse::<std::net::IpAddr>()
                        .map(ipnet::IpNet::from)
                        .map_err(|_| ())
                })
                .map(|network| match network {
                    ipnet::IpNet::V6(v6)
                        if v6.prefix_len() >= 96 && v6.addr().to_ipv4_mapped().is_some() =>
                    {
                        ipnet::IpNet::V4(
                            ipnet::Ipv4Net::new(
                                v6.addr().to_ipv4_mapped().unwrap(),
                                v6.prefix_len() - 96,
                            )
                            .expect("mapped IPv4 prefix"),
                        )
                    }
                    other => other,
                })
                .map_err(|_| ApplicationError::ConnectionInput)
        })
        .transpose()
}
fn matches_query(r: &ConnectionInfo, q: &ConnectionQuery, source: Option<&ipnet::IpNet>) -> bool {
    let context = &r.context;
    let source_matches = source.is_none_or(|network| {
        context.source_ip.is_some_and(|ip| {
            let canonical = match ip {
                std::net::IpAddr::V6(ip) => ip
                    .to_ipv4_mapped()
                    .map(std::net::IpAddr::V4)
                    .unwrap_or(std::net::IpAddr::V6(ip)),
                ip => ip,
            };
            network.contains(&ip) || network.contains(&canonical)
        })
    });
    source_matches
        && q.id.as_ref().is_none_or(|id| *id == r.id)
        && q.source_port
            .is_none_or(|port| context.source_port == Some(port))
        && q.destination.as_ref().is_none_or(|host| {
            let needle = host.to_ascii_lowercase();
            context
                .destination_host
                .to_ascii_lowercase()
                .contains(&needle)
                || r.routing
                    .domain
                    .as_ref()
                    .is_some_and(|domain| domain.to_ascii_lowercase().contains(&needle))
        })
        && q.destination_port
            .is_none_or(|port| context.destination_port == port)
        && q.inbound.as_ref().is_none_or(|id| {
            if id == "@unknown" {
                context.inbound.is_none()
            } else {
                context.inbound.as_ref() == Some(id)
            }
        })
        && q.protocol
            .as_ref()
            .is_none_or(|p| context.inbound_protocol.as_ref() == Some(p))
        && q.transport.as_ref().is_none_or(|t| {
            matches!(
                (t.as_str(), context.transport),
                ("tcp", hps_domain::SelectionTransport::Tcp)
                    | ("udp", hps_domain::SelectionTransport::Udp)
            )
        })
        && q.origin.as_ref().is_none_or(|o| {
            matches!(
                (o.as_str(), context.origin),
                ("explicit", hps_domain::FlowOrigin::Explicit)
                    | ("linux_local", hps_domain::FlowOrigin::LinuxLocal)
                    | ("gateway", hps_domain::FlowOrigin::Gateway)
            )
        })
        && q.node.as_ref().is_none_or(|id| {
            if id == "@direct" {
                r.routing.action == hps_domain::RouteAction::Direct
            } else {
                r.choice.as_ref().is_some_and(|c| c.node_id == *id)
            }
        })
        && q.provider.as_ref().is_none_or(|id| {
            r.choice.as_ref().is_some_and(|c| {
                if id == "@static" {
                    c.provider_id.is_none()
                } else {
                    c.provider_id.as_ref() == Some(id)
                }
            })
        })
        && q.group.as_ref().is_none_or(|id| {
            if id == "@none" {
                r.selection.as_ref().is_none_or(|s| s.group.is_none())
            } else {
                r.selection
                    .as_ref()
                    .is_some_and(|s| s.group.as_ref() == Some(id))
            }
        })
        && q.rule.as_ref().is_none_or(|id| {
            if id == "@default" {
                r.routing.rule_id.is_none()
            } else {
                r.routing.rule_id.as_ref() == Some(id)
            }
        })
        && match q.state.as_deref().unwrap_or("active") {
            "active" => r.ended_at_unix_ms.is_none(),
            "all" => true,
            "ended" => r.ended_at_unix_ms.is_some(),
            state => r.state == state,
        }
        && q.error
            .as_ref()
            .is_none_or(|error| r.error_code.as_ref() == Some(error))
        && q.created_after_unix_ms
            .is_none_or(|after| r.created_at_unix_ms >= after)
}

#[cfg(test)]
mod tests;
