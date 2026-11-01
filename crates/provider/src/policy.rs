//! Pure single-source hysteresis policy. Reservations are enforced again in SQLite.
use hps_config::{ProviderConfig, RefillMode};
#[derive(Debug, Default, Clone)]
pub struct Inventory {
    pub current: u32,
    pub projected: u32,
    pub inflight: u32,
    pub demand_active: bool,
}
#[derive(Debug, Default, Clone)]
pub struct ControlState {
    pub refilling: bool,
    pub paused: bool,
    pub next_allowed_at: i64,
    pub used_requests: u32,
    pub reserved_nodes: u32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub reason: &'static str,
    pub refilling: bool,
    pub count: u32,
}
pub fn decide(
    config: &ProviderConfig,
    inventory: &Inventory,
    state: &ControlState,
    now: i64,
    force: bool,
) -> Decision {
    let policy = &config.refill;
    let hold = |reason, refilling| Decision {
        reason,
        refilling,
        count: 0,
    };
    if !config.enabled || !policy.enabled {
        return hold("disabled", false);
    }
    if state.paused {
        return hold("paused", state.refilling);
    }
    if !force && policy.mode == RefillMode::OnDemand && !inventory.demand_active {
        return hold("idle", false);
    }
    if inventory.inflight > 0 {
        return hold("inflight", state.refilling);
    }
    if inventory.projected >= policy.target_nodes {
        return hold("target_satisfied", false);
    }
    let refilling = state.refilling || force || inventory.projected < policy.min_nodes;
    if !refilling {
        return hold("above_minimum", false);
    }
    if state.used_requests >= policy.daily_request_budget
        || state.reserved_nodes >= policy.daily_node_budget
    {
        return hold("budget_exhausted", true);
    }
    if now < state.next_allowed_at {
        return hold("cooldown", true);
    }
    let headroom = policy
        .max_nodes
        .saturating_sub(inventory.current.saturating_add(inventory.inflight));
    let deficit = policy
        .target_nodes
        .saturating_sub(inventory.projected.saturating_add(inventory.inflight));
    let budget = policy
        .daily_node_budget
        .saturating_sub(state.reserved_nodes);
    let count = policy
        .batch_size
        .min(config.count)
        .min(headroom)
        .min(deficit)
        .min(budget);
    if count == 0 {
        return hold("watermark_capacity", true);
    }
    Decision {
        reason: "refill_needed",
        refilling: true,
        count,
    }
}
/// Jitter is deterministic for a task ID, making persisted cooldowns reproducible.
pub fn cooldown(
    config: &ProviderConfig,
    failures: u32,
    retry_after: Option<u64>,
    task_id: i64,
) -> u64 {
    let backoff = config
        .refill
        .backoff_initial_secs
        .saturating_mul(1u64 << failures.saturating_sub(1).min(20))
        .min(config.refill.backoff_max_secs);
    let jitter = (task_id.unsigned_abs() % 21) * backoff / 100;
    config
        .min_interval_secs
        .max(
            backoff
                .saturating_add(jitter)
                .min(config.refill.backoff_max_secs),
        )
        .max(retry_after.unwrap_or(0).min(86400))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> ProviderConfig {
        let mut c = ProviderConfig {
            enabled: true,
            count: 10,
            ..Default::default()
        };
        c.refill.enabled = true;
        c
    }
    #[test]
    fn idle_demand_hysteresis_and_capacity_are_distinct() {
        let c = config();
        let mut inventory = Inventory::default();
        let mut state = ControlState::default();
        assert_eq!(decide(&c, &inventory, &state, 100, false).reason, "idle");
        inventory.demand_active = true;
        let first = decide(&c, &inventory, &state, 100, false);
        assert_eq!(first.count, 5);
        state.refilling = first.refilling;
        inventory.current = 5;
        inventory.projected = 5;
        assert_eq!(decide(&c, &inventory, &state, 100, false).count, 5);
        state.refilling = false;
        assert_eq!(
            decide(&c, &inventory, &state, 100, false).reason,
            "above_minimum"
        );
        inventory.projected = 0;
        inventory.current = 20;
        assert_eq!(
            decide(&c, &inventory, &state, 100, false).reason,
            "watermark_capacity"
        );
        inventory.current = 18;
        assert_eq!(decide(&c, &inventory, &state, 100, false).count, 2);
        inventory.current = 10;
        inventory.projected = 10;
        assert!(!decide(&c, &inventory, &state, 100, false).refilling);
    }
    #[test]
    fn inflight_pause_budgets_and_backoff_never_create_extra_requests() {
        let c = config();
        let mut inventory = Inventory {
            demand_active: true,
            inflight: 5,
            ..Default::default()
        };
        let mut state = ControlState::default();
        assert_eq!(decide(&c, &inventory, &state, 100, true).reason, "inflight");
        inventory.inflight = 0;
        state.paused = true;
        assert_eq!(decide(&c, &inventory, &state, 100, true).reason, "paused");
        state.paused = false;
        state.used_requests = 10;
        assert_eq!(
            decide(&c, &inventory, &state, 100, true).reason,
            "budget_exhausted"
        );
        state.used_requests = 0;
        state.reserved_nodes = 99;
        assert_eq!(decide(&c, &inventory, &state, 100, true).count, 1);
        state.next_allowed_at = 101;
        assert_eq!(decide(&c, &inventory, &state, 100, true).reason, "cooldown");
        assert!(cooldown(&c, 100, None, 12) >= c.min_interval_secs);
        assert_eq!(cooldown(&c, 1, Some(3600), 1), 3600);
    }
    #[test]
    fn maintenance_is_explicit_and_policy_edits_preserve_lease_provenance() {
        let mut c = config();
        c.id = "source".into();
        let before = crate::fingerprint(&c);
        c.refill.mode = RefillMode::Maintain;
        assert_eq!(
            decide(
                &c,
                &Inventory::default(),
                &ControlState::default(),
                100,
                false
            )
            .count,
            5
        );
        assert_eq!(crate::fingerprint(&c), before);
        c.refill.target_nodes = 25;
        assert!(c.refill.validate(c.count, Some(300), 30).is_err());
        c.refill.target_nodes = 10;
        assert!(c.refill.validate(c.count, Some(60), 30).is_err());
    }
}
