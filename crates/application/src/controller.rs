use super::*;
use hps_config::{ProviderConfig, RefillMode};
use hps_domain::{ControllerEvaluation, ProviderControllerStatus};
use hps_provider::policy::{ControlState, Decision, Inventory};
use std::{collections::HashSet, time::Duration};
impl Supervisor {
    pub(crate) fn start_pool_controller(self: &Arc<Self>) {
        if self
            .controller_started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let weak = Arc::downgrade(self);
        let cancel = self.provider_cancel.clone();
        self.provider_tasks.spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_secs(1));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;_=cancel.cancelled()=>break,_=timer.tick()=>{}}
                let Some(service) = weak.upgrade() else {
                    break;
                };
                let ids = service
                    .current()
                    .config
                    .providers
                    .iter()
                    .filter(|p| p.enabled && p.refill.enabled)
                    .map(|p| p.id.clone())
                    .collect::<Vec<_>>();
                for id in ids {
                    if cancel.is_cancelled() {
                        break;
                    }
                    let _ = service.evaluate_controller_inner(id, false).await;
                }
            }
        });
    }
    async fn controller_context(
        &self,
        id: &str,
        force: bool,
    ) -> Result<(ProviderConfig, ProviderControllerStatus, Decision), ApplicationError> {
        let current = self.current();
        let config = current
            .config
            .providers
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or(ApplicationError::ProviderUnavailable)?;
        let now = current
            .engine
            .dynamic_pool()
            .logical_now()
            .min(i64::MAX as u64) as i64;
        let record = self.store.controller_snapshot_at(id.into(), now).await?;
        let leases = self.store.active_source_leases(id.into()).await?;
        let fp = hps_provider::fingerprint(&config);
        let retirement = config
            .min_remaining_secs
            .max(current.config.proxy.limits.min_remaining_lifetime_secs)
            .max(if config.refill.enabled {
                config.refill.retire_before_secs
            } else {
                0
            });
        let cutoff = now.saturating_add(retirement.min(i64::MAX as u64) as i64);
        let future = cutoff.saturating_add(config.refill.forecast_secs as i64);
        let mut seen = HashSet::new();
        let plans = current
            .engine
            .probe_plans()
            .into_iter()
            .map(|p| (p.config().id.clone(), p))
            .collect::<std::collections::HashMap<_, _>>();
        let mut pending_health = 0u32;
        let mut projected_pending = 0u32;
        let mut health_source_blocked = false;
        let deadlines = leases
            .iter()
            .filter(|l| config.enabled && l.fingerprint == fp && l.candidate.expires_at > cutoff)
            .filter(|l| {
                seen.insert((
                    format!("{:?}", l.candidate.upstream.protocol),
                    l.candidate.upstream.host.clone(),
                    l.candidate.upstream.port,
                ))
            })
            .filter_map(|l| {
                let (ready, pending, blocked) = plans.get(&l.candidate.upstream.id).map_or(
                    (
                        !current.config.health.enabled,
                        current.config.health.enabled,
                        false,
                    ),
                    |plan| self.plan_health(plan, &current),
                );
                health_source_blocked |= blocked;
                if pending {
                    pending_health += 1;
                    if l.candidate.expires_at > future {
                        projected_pending += 1;
                    }
                }
                (ready || pending).then_some(l.candidate.expires_at)
            })
            .take(if config.refill.enabled {
                config.refill.max_nodes as usize
            } else {
                100000
            })
            .collect::<Vec<_>>();
        let (active, age) = current.engine.source_demand(&config.id);
        let demand_active = active
            || age.is_some_and(|age| age <= Duration::from_secs(config.refill.idle_timeout_secs));
        let clock_wrong = now > hps_provider::unix_now().saturating_add(5);
        let inventory = Inventory {
            current: deadlines.len() as u32,
            projected: deadlines
                .iter()
                .filter(|deadline| **deadline > future)
                .count() as u32,
            inflight: record.inflight.as_ref().map_or(0, |t| t.requested_count),
            demand_active,
        };
        let health_block = if config.enabled && config.refill.enabled {
            self.health_purchase_block()
                .await?
                .or(if health_source_blocked {
                    Some("health_source_configuration")
                } else {
                    None
                })
        } else {
            None
        };
        let state = ControlState {
            refilling: record.refilling,
            paused: record.paused_reason.is_some() || clock_wrong || health_block.is_some(),
            next_allowed_at: record.next_allowed_at,
            used_requests: record.used_requests,
            reserved_nodes: record.reserved_nodes,
        };
        let mut decision = hps_provider::policy::decide(&config, &inventory, &state, now, force);
        if let Some(reason) = health_block {
            decision.reason = reason;
        }
        if pending_health > 0
            && decision.count == 0
            && health_block.is_none()
            && !clock_wrong
            && ["target_satisfied", "above_minimum", "watermark_capacity"]
                .contains(&decision.reason)
        {
            decision.reason = "health_pending";
        }
        if clock_wrong {
            decision.reason = "clock_rollback";
        }
        let status = ProviderControllerStatus {
            provider_id: id.into(),
            enabled: config.enabled && config.refill.enabled,
            mode: match config.refill.mode {
                RefillMode::OnDemand => "on_demand",
                RefillMode::Maintain => "maintain",
            }
            .into(),
            min_nodes: config.refill.min_nodes,
            target_nodes: config.refill.target_nodes,
            max_nodes: config.refill.max_nodes,
            current_inventory: inventory.current.saturating_sub(pending_health),
            pending_health_nodes: pending_health,
            projected_inventory: inventory.projected.saturating_sub(projected_pending),
            inflight_nodes: inventory.inflight,
            demand_active,
            refilling: record.refilling,
            decision: decision.reason.into(),
            planned_count: decision.count,
            day_started_at: record.day * 86400,
            resets_at: record.day.saturating_add(1) * 86400,
            used_requests: record.used_requests,
            request_budget: config.refill.daily_request_budget,
            reserved_nodes: record.reserved_nodes,
            node_budget: config.refill.daily_node_budget,
            next_allowed_at: record.next_allowed_at,
            failure_count: record.failure_count,
            paused_reason: if clock_wrong {
                Some("clock_rollback".into())
            } else {
                record.paused_reason
            },
            last_task_id: record.last_task_id,
            acknowledged_at: record.acknowledged_at,
        };
        Ok((config, status, decision))
    }
    pub async fn provider_controller(
        &self,
        id: &str,
    ) -> Result<ProviderControllerStatus, ApplicationError> {
        Ok(self.controller_context(id, false).await?.1)
    }
    pub async fn evaluate_controller(
        self: &Arc<Self>,
        id: String,
    ) -> Result<ControllerEvaluation, ApplicationError> {
        let service = self.clone();
        tokio::spawn(async move { service.evaluate_controller_inner(id, true).await })
            .await
            .map_err(|_| ApplicationError::Worker)?
    }
    async fn evaluate_controller_inner(
        self: &Arc<Self>,
        id: String,
        force: bool,
    ) -> Result<ControllerEvaluation, ApplicationError> {
        let _operation = self.operation.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        let (mut config, _, decision) = self.controller_context(&id, force).await?;
        self.store
            .set_refilling(id.clone(), decision.refilling)
            .await?;
        let task = if decision.count > 0 {
            let forecast = config.refill.forecast_secs;
            config.min_remaining_secs = config
                .min_remaining_secs
                .max(config.refill.retire_before_secs)
                .max(
                    self.current()
                        .config
                        .proxy
                        .limits
                        .min_remaining_lifetime_secs,
                )
                .saturating_add(forecast);
            config.refill.forecast_secs = 0;
            config.count = decision.count;
            config.refill.batch_size = config.refill.batch_size.min(decision.count);
            Some(
                self.accept_extraction_locked(
                    config,
                    format!("auto-{}", uuid::Uuid::new_v4()),
                    "automatic",
                )
                .await?,
            )
        } else {
            None
        };
        Ok(ControllerEvaluation {
            status: self.provider_controller(&id).await?,
            task,
        })
    }
    pub async fn pause_controller(
        &self,
        id: String,
        paused: bool,
    ) -> Result<ProviderControllerStatus, ApplicationError> {
        let _operation = self.operation.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(EngineError::ShuttingDown.into());
        }
        if !self.current().config.providers.iter().any(|p| p.id == id) {
            return Err(ApplicationError::ProviderUnavailable);
        }
        self.store.pause_provider(id.clone(), paused).await?;
        self.provider_controller(&id).await
    }
}
