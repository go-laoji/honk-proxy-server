//! Shared durable extraction reservations and source cooldowns.
use super::*;
use hps_config::{ProviderConfig, RefillConfig};
use hps_domain::ExtractionTask;
use hps_provider::FetchError;

#[derive(Debug, Clone)]
pub struct ControllerSnapshot {
    pub provider_id: String,
    pub day: i64,
    pub used_requests: u32,
    pub reserved_nodes: u32,
    pub refilling: bool,
    pub next_allowed_at: i64,
    pub failure_count: u32,
    pub paused_reason: Option<String>,
    pub last_task_id: Option<i64>,
    pub acknowledged_at: Option<i64>,
    pub inflight: Option<ExtractionTask>,
}
pub(super) fn budget_day(c: &Connection, now: i64) -> Result<i64, StorageError> {
    let observed: i64 = c.query_row(
        "SELECT observed_at FROM provider_clock WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    Ok(now.max(observed).div_euclid(86400))
}
pub(super) fn reserve(
    c: &Connection,
    config: &ProviderConfig,
    now: i64,
) -> Result<(), StorageError> {
    c.execute(
        "INSERT OR IGNORE INTO provider_control(provider_id,updated_at) VALUES(?1,?2)",
        params![config.id, now],
    )?;
    let (pause, next): (Option<String>, i64) = c.query_row(
        "SELECT paused_reason,next_allowed_at FROM provider_control WHERE provider_id=?1",
        [&config.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if pause.is_some() {
        return Err(StorageError::ProviderPaused);
    }
    if next > now {
        return Err(StorageError::ProviderRateLimited);
    }
    let day = budget_day(c, now)?;
    let (requests,nodes):(u32,u32)=c.query_row("SELECT requests,reserved_nodes FROM provider_daily_usage WHERE provider_id=?1 AND day=?2",params![config.id,day],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.unwrap_or_default();
    if requests >= config.refill.daily_request_budget
        || nodes.saturating_add(config.count) > config.refill.daily_node_budget
    {
        return Err(StorageError::ProviderBudgetExceeded);
    }
    c.execute("INSERT INTO provider_daily_usage(provider_id,day,requests,reserved_nodes) VALUES(?1,?2,1,?3) ON CONFLICT(provider_id,day) DO UPDATE SET requests=requests+1,reserved_nodes=reserved_nodes+excluded.reserved_nodes",params![config.id,day,config.count])?;
    c.execute(
        "UPDATE provider_clock SET observed_at=max(observed_at,?1) WHERE singleton=1",
        [now],
    )?;
    c.execute("DELETE FROM provider_daily_usage WHERE day<?1", [day - 400])?;
    Ok(())
}
pub(super) fn success(c: &Connection, provider: &str, now: i64) -> Result<(), StorageError> {
    c.execute("UPDATE provider_control SET failure_count=0,paused_reason=CASE WHEN paused_reason='manual' THEN paused_reason END,updated_at=?2 WHERE provider_id=?1",params![provider,now])?;
    Ok(())
}
pub(super) fn failure(
    c: &Connection,
    id: i64,
    error: &FetchError,
    now: i64,
) -> Result<(), StorageError> {
    let (provider, policy, min_interval): (String, String, i64) = c.query_row(
        "SELECT provider_id,policy_json,min_interval_secs FROM provider_tasks WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let refill: RefillConfig = serde_json::from_str(&policy).map_err(|_| StorageError::Invalid)?;
    let failures: u32 = c.query_row(
        "SELECT failure_count FROM provider_control WHERE provider_id=?1",
        [&provider],
        |r| r.get(0),
    )?;
    let failures = failures.saturating_add(1);
    let pause = if error.outcome_unknown
        || [
            "quota_exhausted",
            "whitelist_rejected",
            "vendor_auth_or_parameters",
            "http_auth_or_whitelist",
            "order_unavailable",
            "no_admissible_leases",
            "leases_too_short",
        ]
        .contains(&error.code)
    {
        Some(error.code)
    } else if failures >= refill.failure_limit {
        Some("consecutive_failures")
    } else {
        None
    };
    let config = ProviderConfig {
        min_interval_secs: min_interval.max(1) as u64,
        refill,
        ..Default::default()
    };
    let delay = hps_provider::policy::cooldown(&config, failures, error.retry_after_secs, id)
        .min(i64::MAX as u64) as i64;
    c.execute("UPDATE provider_control SET failure_count=?2,paused_reason=CASE WHEN paused_reason='manual' THEN paused_reason ELSE ?3 END,next_allowed_at=max(next_allowed_at,?4),updated_at=?5 WHERE provider_id=?1",params![provider,failures,pause,now.saturating_add(delay),now])?;
    Ok(())
}
impl Store {
    pub async fn controller_snapshot(
        &self,
        provider_id: String,
    ) -> Result<ControllerSnapshot, StorageError> {
        self.controller_snapshot_at(provider_id, unix_now()).await
    }
    pub async fn controller_snapshot_at(
        &self,
        provider_id: String,
        now: i64,
    ) -> Result<ControllerSnapshot, StorageError> {
        self.call(move|c| {
            let day=budget_day(c,now)?;
            let usage=c.query_row("SELECT requests,reserved_nodes FROM provider_daily_usage WHERE provider_id=?1 AND day=?2",params![provider_id,day],|r|Ok((r.get::<_,u32>(0)?,r.get::<_,u32>(1)?))).optional()?.unwrap_or_default();
            let row=c.query_row("SELECT refilling,next_allowed_at,failure_count,paused_reason,last_task_id,acknowledged_at FROM provider_control WHERE provider_id=?1",[&provider_id],|r|Ok((r.get::<_,bool>(0)?,r.get::<_,i64>(1)?,r.get::<_,u32>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<i64>>(4)?,r.get::<_,Option<i64>>(5)?))).optional()?.unwrap_or_default();
            let inflight=c.query_row(&format!("SELECT {} FROM provider_tasks WHERE provider_id=?1 AND state IN ('accepted','requesting') ORDER BY id DESC LIMIT 1",super::provider::TASK_COLUMNS),[&provider_id],super::provider::task).optional()?;
            Ok(ControllerSnapshot{provider_id,day,used_requests:usage.0,reserved_nodes:usage.1,refilling:row.0,next_allowed_at:row.1,failure_count:row.2,paused_reason:row.3,last_task_id:row.4,acknowledged_at:row.5,inflight})
        }).await
    }
    pub async fn set_refilling(
        &self,
        provider_id: String,
        refilling: bool,
    ) -> Result<(), StorageError> {
        self.call(move|c|{c.execute("INSERT INTO provider_control(provider_id,refilling,updated_at) VALUES(?1,?2,?3) ON CONFLICT(provider_id) DO UPDATE SET refilling=excluded.refilling,updated_at=excluded.updated_at WHERE refilling!=excluded.refilling",params![provider_id,refilling,unix_now()])?;Ok(())}).await
    }
    /// Resume acknowledges a previously uncertain result; budgets and Retry-After remain intact.
    pub async fn pause_provider(
        &self,
        provider_id: String,
        paused: bool,
    ) -> Result<(), StorageError> {
        self.call(move|c| {
            let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let now=unix_now();
            // An acknowledgement belongs to the current terminal task. Resuming an
            // in-flight request cannot acknowledge a future unknown outcome.
            let finished:Option<i64>=if paused {None}else{tx.query_row("SELECT t.finished_at FROM provider_tasks t JOIN provider_control p ON p.last_task_id=t.id WHERE p.provider_id=?1 AND t.state NOT IN ('accepted','requesting')",[&provider_id],|r|r.get(0)).optional()?.flatten()};
            let acknowledged=finished.map(|at|at.max(now));
            tx.execute("INSERT INTO provider_control(provider_id,paused_reason,acknowledged_at,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(provider_id) DO UPDATE SET paused_reason=excluded.paused_reason,acknowledged_at=CASE WHEN excluded.paused_reason IS NULL THEN excluded.acknowledged_at ELSE acknowledged_at END,failure_count=CASE WHEN excluded.paused_reason IS NULL THEN 0 ELSE failure_count END,updated_at=excluded.updated_at",params![provider_id,if paused{Some("manual")}else{None},acknowledged,now])?;
            tx.commit()?;Ok(())
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_config::ProviderKind;
    fn config() -> ProviderConfig {
        let mut c = ProviderConfig {
            id: "source".into(),
            kind: ProviderKind::Text,
            endpoint: "http://localhost/extract".into(),
            enabled: true,
            count: 3,
            ttl_secs: Some(300),
            min_interval_secs: 1,
            ..Default::default()
        };
        c.refill.daily_request_budget = 2;
        c.refill.daily_node_budget = 6;
        c
    }
    async fn accept(
        store: &Store,
        c: &ProviderConfig,
        key: &str,
        trigger: &str,
    ) -> Result<(ExtractionTask, bool), StorageError> {
        store
            .begin_extraction_with_trigger(
                c.clone(),
                key.into(),
                hps_provider::fingerprint(c),
                trigger.into(),
            )
            .await
    }
    async fn cool(store: &Store) {
        store
            .call(|c| {
                c.execute("UPDATE provider_tasks SET started_at=started_at-1000", [])?;
                c.execute("UPDATE provider_control SET next_allowed_at=0", [])?;
                Ok(())
            })
            .await
            .unwrap();
    }
    fn error(code: &'static str, unknown: bool) -> FetchError {
        FetchError {
            code,
            outcome_unknown: unknown,
            retry_after_secs: None,
        }
    }
    #[tokio::test]
    async fn manual_and_automatic_reserve_one_budget_and_history_pruning_cannot_refund_it() {
        let store = Store::open(None, DaemonConfig::default()).unwrap();
        let c = config();
        let (a, new) = accept(&store, &c, "a", "manual").await.unwrap();
        assert!(new);
        let (merged, new) = accept(&store, &c, "b", "automatic").await.unwrap();
        assert_eq!(merged.id, a.id);
        assert!(!new);
        let snapshot = store.controller_snapshot(c.id.clone()).await.unwrap();
        assert_eq!((snapshot.used_requests, snapshot.reserved_nodes), (1, 3));
        assert!(snapshot.inflight.is_some());
        store
            .fail_extraction(a.id, error("empty_result", false))
            .await
            .unwrap();
        cool(&store).await;
        let (b, _) = accept(&store, &c, "b", "automatic").await.unwrap();
        assert_eq!(b.trigger, "automatic");
        store
            .fail_extraction(b.id, error("empty_result", false))
            .await
            .unwrap();
        cool(&store).await;
        store
            .call(|c| {
                c.execute("DELETE FROM provider_tasks", [])?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(matches!(
            accept(&store, &c, "c", "manual").await,
            Err(StorageError::ProviderBudgetExceeded)
        ));
        let snapshot = store.controller_snapshot(c.id.clone()).await.unwrap();
        assert_eq!((snapshot.used_requests, snapshot.reserved_nodes), (2, 6));
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn unknown_outcomes_pause_and_acknowledgement_preserves_cooldown_and_budget_across_restart()
     {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let c = config();
        let (task, _) = accept(&store, &c, "uncertain", "automatic").await.unwrap();
        store.extraction_requesting(task.id).await.unwrap();
        store
            .fail_extraction(task.id, error("response_outcome_unknown", true))
            .await
            .unwrap();
        let snapshot = store.controller_snapshot(c.id.clone()).await.unwrap();
        assert_eq!(
            snapshot.paused_reason.as_deref(),
            Some("response_outcome_unknown")
        );
        assert!(matches!(
            accept(&store, &c, "new-id", "manual").await,
            Err(StorageError::ProviderPaused)
        ));
        store.close().await.unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let snapshot = store.controller_snapshot(c.id.clone()).await.unwrap();
        assert!(snapshot.paused_reason.is_some());
        assert_eq!(snapshot.reserved_nodes, 3);
        store.pause_provider(c.id.clone(), false).await.unwrap();
        let resumed = store.controller_snapshot(c.id.clone()).await.unwrap();
        assert!(resumed.paused_reason.is_none());
        assert_eq!(resumed.next_allowed_at, snapshot.next_allowed_at);
        assert_eq!(resumed.used_requests, 1);
        assert!(resumed.acknowledged_at.is_some());
        assert!(matches!(
            accept(&store, &c, "new-id", "manual").await,
            Err(StorageError::ProviderRateLimited)
        ));
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn retry_after_no_quota_and_budget_day_watermark_are_conservative() {
        let store = Store::open(None, DaemonConfig::default()).unwrap();
        let c = config();
        let (task, _) = accept(&store, &c, "limited", "automatic").await.unwrap();
        store
            .fail_extraction(
                task.id,
                FetchError {
                    retry_after_secs: Some(3600),
                    ..error("rate_limited", false)
                },
            )
            .await
            .unwrap();
        let before = store.controller_snapshot(c.id.clone()).await.unwrap();
        assert!(before.paused_reason.is_none());
        assert!(before.next_allowed_at >= unix_now() + 3599);
        cool(&store).await;
        let (task, _) = accept(&store, &c, "quota", "automatic").await.unwrap();
        store
            .fail_extraction(task.id, error("quota_exhausted", false))
            .await
            .unwrap();
        assert_eq!(
            store
                .controller_snapshot(c.id.clone())
                .await
                .unwrap()
                .paused_reason
                .as_deref(),
            Some("quota_exhausted")
        );
        store
            .observe_lease_clock((before.day + 1) * 86400 + 5)
            .await
            .unwrap();
        let tomorrow = store.controller_snapshot(c.id.clone()).await.unwrap();
        assert_eq!(tomorrow.day, before.day + 1);
        assert_eq!(tomorrow.used_requests, 0);
        assert!(tomorrow.paused_reason.is_some());
        store
            .observe_lease_clock((before.day - 1) * 86400)
            .await
            .unwrap();
        assert_eq!(
            store.controller_snapshot(c.id.clone()).await.unwrap().day,
            before.day + 1
        );
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn schema_four_requests_are_backfilled_into_budget_and_uncertain_pause() {
        let dir = tempfile::tempdir().unwrap();
        let c = Connection::open(dir.path().join("hps.sqlite3")).unwrap();
        c.execute_batch(include_str!("../migrations/0001_configuration.up.sql"))
            .unwrap();
        c.execute_batch(include_str!("../migrations/0002_known_good.up.sql"))
            .unwrap();
        c.execute_batch(include_str!("../migrations/0003_provider_leases.up.sql"))
            .unwrap();
        c.execute_batch(include_str!("../migrations/0004_lease_clock.up.sql"))
            .unwrap();
        c.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at INTEGER NOT NULL);INSERT INTO schema_migrations VALUES(4,0)").unwrap();
        c.execute(
            "INSERT INTO provider_clock(singleton,observed_at,clean_shutdown) VALUES(1,?1,1)",
            [unix_now()],
        )
        .unwrap();
        for (key, state) in [("legacy-a", "failed"), ("legacy-b", "outcome_unknown")] {
            c.execute("INSERT INTO provider_tasks(provider_id,request_id,fingerprint,state,requested_count,started_at) VALUES('source',?1,?2,?3,3,?4)",params![key,"a".repeat(64),state,unix_now()]).unwrap();
        }
        drop(c);
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let snapshot = store.controller_snapshot("source".into()).await.unwrap();
        assert_eq!((snapshot.used_requests, snapshot.reserved_nodes), (2, 6));
        assert_eq!(snapshot.paused_reason.as_deref(), Some("outcome_unknown"));
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn resume_does_not_acknowledge_a_future_unknown_outcome_or_a_different_task() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let c = config();
        let (task, _) = accept(&store, &c, "first", "automatic").await.unwrap();
        store.extraction_requesting(task.id).await.unwrap();
        store.pause_provider(c.id.clone(), true).await.unwrap();
        store.pause_provider(c.id.clone(), false).await.unwrap();
        assert!(
            store
                .controller_snapshot(c.id.clone())
                .await
                .unwrap()
                .acknowledged_at
                .is_none()
        );
        store.close().await.unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        assert_eq!(
            store
                .controller_snapshot(c.id.clone())
                .await
                .unwrap()
                .paused_reason
                .as_deref(),
            Some("process_interrupted")
        );
        store.pause_provider(c.id.clone(), false).await.unwrap();
        store.close().await.unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        assert!(
            store
                .controller_snapshot(c.id.clone())
                .await
                .unwrap()
                .paused_reason
                .is_none()
        );
        cool(&store).await;
        let (task, _) = accept(&store, &c, "second", "automatic").await.unwrap();
        store.extraction_requesting(task.id).await.unwrap();
        assert!(
            store
                .controller_snapshot(c.id.clone())
                .await
                .unwrap()
                .acknowledged_at
                .is_none()
        );
        store.close().await.unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        assert_eq!(
            store
                .controller_snapshot(c.id.clone())
                .await
                .unwrap()
                .paused_reason
                .as_deref(),
            Some("process_interrupted")
        );
        store.close().await.unwrap();
    }
}
