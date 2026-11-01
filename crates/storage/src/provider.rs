use super::*;
use hps_config::{LeaseMode, ProviderConfig};
use hps_domain::{ExtractionTask, LeaseInfo};
use hps_provider::{FetchError, FetchResult, LeaseCandidate};

#[derive(Debug, Clone)]
pub struct StoredLease {
    pub id: i64,
    pub batch_id: i64,
    pub provider_id: String,
    pub fingerprint: String,
    pub candidate: LeaseCandidate,
    pub received_at: i64,
    pub state: String,
}
impl StoredLease {
    pub fn info(&self, now: i64) -> LeaseInfo {
        LeaseInfo {
            id: self.id,
            batch_id: self.batch_id,
            provider_id: self.provider_id.clone(),
            config_fingerprint: self.fingerprint.clone(),
            upstream_id: self.candidate.upstream.id.clone(),
            protocol: format!("{:?}", self.candidate.upstream.protocol).to_lowercase(),
            host: self.candidate.upstream.host.clone(),
            port: self.candidate.upstream.port,
            received_at: self.received_at,
            expires_at: self.candidate.expires_at,
            expiry_basis: self.candidate.expiry_basis.clone(),
            state: if self.state == "active" && self.candidate.expires_at <= now {
                "expired".into()
            } else {
                self.state.clone()
            },
            region: self.candidate.region.clone(),
        }
    }
}
pub(super) const TASK_COLUMNS: &str = "id,provider_id,request_id,fingerprint,state,requested_count,started_at,finished_at,added,rejected,removed,quota,error_code,retry_after_secs,trigger";
pub(super) fn task(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExtractionTask> {
    Ok(ExtractionTask {
        id: row.get(0)?,
        trigger: row.get(14)?,
        provider_id: row.get(1)?,
        request_id: row.get(2)?,
        config_fingerprint: row.get(3)?,
        state: row.get(4)?,
        requested_count: row.get(5)?,
        started_at: row.get(6)?,
        finished_at: row.get(7)?,
        added: row.get::<_, u32>(8)? as usize,
        rejected: row.get::<_, u32>(9)? as usize,
        removed: row.get::<_, u32>(10)? as usize,
        quota: row.get::<_, Option<i64>>(11)?.map(|n| n as u64),
        error_code: row.get(12)?,
        retry_after_secs: row.get::<_, Option<i64>>(13)?.map(|n| n as u64),
    })
}
fn get_task(connection: &Connection, id: i64) -> Result<ExtractionTask, StorageError> {
    connection
        .query_row(
            &format!("SELECT {TASK_COLUMNS} FROM provider_tasks WHERE id=?1"),
            [id],
            task,
        )
        .optional()?
        .ok_or(StorageError::NotFound)
}
fn lease(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredLease> {
    let document: String = row.get(4)?;
    let candidate: LeaseCandidate = serde_json::from_str(&document).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(StoredLease {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        provider_id: row.get(2)?,
        fingerprint: row.get(3)?,
        candidate,
        received_at: row.get(5)?,
        state: row.get(6)?,
    })
}
fn prune_history(c: &Connection, now: i64) -> Result<(), StorageError> {
    c.execute("DELETE FROM provider_leases WHERE expires_at<?1 OR id IN (SELECT id FROM provider_leases WHERE state!='active' OR expires_at<=?2 ORDER BY id DESC LIMIT -1 OFFSET 100000)",params![now-30*86400,now])?;
    c.execute(
        "DELETE FROM provider_batches WHERE task_id NOT IN (SELECT batch_id FROM provider_leases)",
        [],
    )?;
    c.execute("DELETE FROM provider_tasks WHERE id NOT IN (SELECT task_id FROM provider_batches) AND state NOT IN ('accepted','requesting') AND id NOT IN (SELECT id FROM provider_tasks ORDER BY id DESC LIMIT 10000)",[])?;
    Ok(())
}
impl Store {
    pub async fn observe_lease_clock(&self, now: i64) -> Result<(), StorageError> {
        self.call(move |c| {
            c.execute(
                "UPDATE provider_clock SET observed_at=max(observed_at,?1) WHERE singleton=1",
                [now],
            )?;
            Ok(())
        })
        .await
    }
    /// Returns (task, newly accepted). Same ID or any in-flight request is merged.
    pub async fn begin_extraction(
        &self,
        config: ProviderConfig,
        request_id: String,
        fingerprint: String,
    ) -> Result<(ExtractionTask, bool), StorageError> {
        self.begin_extraction_with_trigger(config, request_id, fingerprint, "manual".into())
            .await
    }
    pub async fn begin_extraction_with_trigger(
        &self,
        config: ProviderConfig,
        request_id: String,
        fingerprint: String,
        trigger: String,
    ) -> Result<(ExtractionTask, bool), StorageError> {
        if !["manual", "automatic"].contains(&trigger.as_str()) {
            return Err(StorageError::Invalid);
        }
        if request_id.is_empty()
            || request_id.len() > 64
            || !request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(StorageError::Invalid);
        }
        self.call(move |connection| {
            let tx=connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let previous=tx.query_row(&format!("SELECT {TASK_COLUMNS} FROM provider_tasks WHERE provider_id=?1 AND (request_id=?2 OR state IN ('accepted','requesting')) ORDER BY CASE WHEN request_id=?2 THEN 0 ELSE 1 END,id DESC LIMIT 1"),params![config.id,request_id],task).optional()?;
            if let Some(previous)=previous {return Ok((previous,false));}
            let now=unix_now();
            prune_history(&tx,now)?;
            super::controller::reserve(&tx,&config,now)?;
            let last:Option<i64>=tx.query_row("SELECT max(started_at) FROM provider_tasks WHERE provider_id=?1",[&config.id],|r|r.get(0))?;
            if last.is_some_and(|last|now.saturating_sub(last)<config.min_interval_secs as i64) {return Err(StorageError::ProviderRateLimited);}
            let size:i64=tx.query_row("SELECT count(*) FROM provider_leases WHERE expires_at>?1 AND state='active'",[now],|r|r.get(0))?;
            let reserved:i64=tx.query_row("SELECT coalesce(sum(requested_count),0) FROM provider_tasks WHERE state IN ('accepted','requesting')",[],|r|r.get(0))?;
            if size+reserved+i64::from(config.count)>100000 {return Err(StorageError::PoolFull);}
            tx.execute("INSERT INTO provider_tasks(provider_id,request_id,fingerprint,state,requested_count,started_at,trigger,policy_json,min_interval_secs) VALUES(?1,?2,?3,'accepted',?4,?5,?6,?7,?8)",params![config.id,request_id,fingerprint,config.count,now,trigger,serde_json::to_string(&config.refill).map_err(|_|StorageError::Invalid)?,config.min_interval_secs as i64])?;
            let id=tx.last_insert_rowid();
            tx.execute("UPDATE provider_control SET last_task_id=?2,acknowledged_at=NULL,next_allowed_at=max(next_allowed_at,?3),updated_at=?4 WHERE provider_id=?1",params![config.id,id,now.saturating_add(config.min_interval_secs as i64),now])?;
            let result=get_task(&tx,id)?;tx.commit()?;Ok((result,true))
        }).await
    }
    pub async fn existing_extraction(
        &self,
        provider_id: String,
        request_id: String,
    ) -> Result<Option<ExtractionTask>, StorageError> {
        self.call(move|c|Ok(c.query_row(&format!("SELECT {TASK_COLUMNS} FROM provider_tasks WHERE provider_id=?1 AND (request_id=?2 OR state IN ('accepted','requesting')) ORDER BY CASE WHEN request_id=?2 THEN 0 ELSE 1 END,id DESC LIMIT 1"),params![provider_id,request_id],task).optional()?)).await
    }
    pub async fn active_source_leases(
        &self,
        provider_id: String,
    ) -> Result<Vec<StoredLease>, StorageError> {
        self.call(move|c|Ok(c.prepare("SELECT id,batch_id,provider_id,fingerprint,document,received_at,state FROM provider_leases WHERE provider_id=?1 AND state='active' AND expires_at>?2 ORDER BY expires_at DESC,id DESC LIMIT 100000")?.query_map(params![provider_id,unix_now()],lease)?.collect::<Result<Vec<_>,_>>()?)).await
    }
    pub async fn extraction_requesting(&self, id: i64) -> Result<(), StorageError> {
        self.call(move |c| {
            if c.execute(
                "UPDATE provider_tasks SET state='requesting' WHERE id=?1 AND state='accepted'",
                [id],
            )? != 1
            {
                return Err(StorageError::Conflict);
            }
            Ok(())
        })
        .await
    }
    pub async fn extraction_task(&self, id: i64) -> Result<ExtractionTask, StorageError> {
        self.call(move |c| get_task(c, id)).await
    }
    pub async fn extraction_tasks(
        &self,
        provider_id: Option<String>,
    ) -> Result<Vec<ExtractionTask>, StorageError> {
        self.call(move|c|Ok(c.prepare(&format!("SELECT {TASK_COLUMNS} FROM provider_tasks WHERE (?1 IS NULL OR provider_id=?1) ORDER BY id DESC LIMIT 50"))?.query_map([provider_id],task)?.collect::<Result<Vec<_>,_>>()?)).await
    }
    pub async fn fail_extraction(&self, id: i64, error: FetchError) -> Result<(), StorageError> {
        self.call(move|c|{
            let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let now=unix_now();
            let changed=tx.execute("UPDATE provider_tasks SET state=?2,error_code=?3,retry_after_secs=?4,finished_at=?5 WHERE id=?1 AND state IN ('accepted','requesting')",params![id,if error.outcome_unknown{"outcome_unknown"}else{"failed"},error.code,error.retry_after_secs.map(|n|n.min(i64::MAX as u64) as i64),now])?;
            if changed>0 {super::controller::failure(&tx,id,&error,now)?;}
            tx.commit()?;Ok(())
        }).await
    }
    pub async fn finish_extraction(
        &self,
        id: i64,
        config: ProviderConfig,
        result: FetchResult,
        publish: bool,
    ) -> Result<ExtractionTask, StorageError> {
        self.call(move|c| {
            let tx=c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;let accepted=get_task(&tx,id)?;
            if accepted.state!="requesting" {return Err(StorageError::Conflict);}
            let now=unix_now();
            if !publish {
                tx.execute("UPDATE provider_tasks SET state='discarded',finished_at=?2,error_code='source_configuration_changed' WHERE id=?1",params![id,now])?;
                super::controller::success(&tx,&config.id,now)?;
                let result=get_task(&tx,id)?;tx.commit()?;return Ok(result);
            }
            let mut rejected=result.rejected;
            let nodes=result.nodes.into_iter().filter(|node|{let valid=node.expires_at>now.saturating_add(config.min_remaining_secs as i64);if !valid {rejected+=1;} valid}).collect::<Vec<_>>();
            let old:i64=tx.query_row("SELECT count(DISTINCT json_extract(document,'$.upstream.protocol') || '|' || json_extract(document,'$.upstream.host') || '|' || json_extract(document,'$.upstream.port')) FROM provider_leases WHERE provider_id=?1 AND state='active' AND expires_at>?2",params![config.id,now],|r|r.get(0))?;
            if nodes.is_empty() || (config.mode==LeaseMode::ReplaceSnapshot && nodes.len()*100<(old as usize)*usize::from(config.minimum_snapshot_percent)) {
                let code=if nodes.is_empty(){"leases_too_short"}else{"snapshot_or_empty_result_protection"};
                tx.execute("UPDATE provider_tasks SET state='failed',finished_at=?2,error_code=?4,rejected=?3 WHERE id=?1",params![id,now,rejected as i64,code])?;
                super::controller::failure(&tx,id,&FetchError{code,outcome_unknown:false,retry_after_secs:None},now)?;
                let result=get_task(&tx,id)?;tx.commit()?;return Ok(result);
            }
            let removed=if config.mode==LeaseMode::ReplaceSnapshot {tx.execute("UPDATE provider_leases SET state='revoked' WHERE provider_id=?1 AND state='active' AND expires_at>?2",params![config.id,now])?}else{0};
            tx.execute("INSERT INTO provider_batches(task_id,provider_id,fingerprint,received_at,mode) VALUES(?1,?2,?3,?4,?5)",params![id,config.id,accepted.config_fingerprint,now,if config.mode==LeaseMode::ReplaceSnapshot{"replace_snapshot"}else{"append_leases"}])?;
            let added=nodes.len();
            for mut node in nodes {
                tx.execute("INSERT INTO provider_leases(batch_id,provider_id,fingerprint,document,received_at,expires_at,state) VALUES(?1,?2,?3,'{}',?4,?5,'active')",params![id,config.id,accepted.config_fingerprint,now,node.expires_at])?;
                let lease_id=tx.last_insert_rowid();node.upstream.id=format!("lease:{lease_id}");
                let document=serde_json::to_string(&node).map_err(|_|StorageError::Invalid)?;
                tx.execute("UPDATE provider_leases SET document=?2 WHERE id=?1",params![lease_id,document])?;
            }
            tx.execute("UPDATE provider_tasks SET state='succeeded',finished_at=?2,added=?3,rejected=?4,removed=?5,quota=?6 WHERE id=?1",params![id,now,added as i64,rejected as i64,removed as i64,result.quota.map(|n|n.min(i64::MAX as u64) as i64)])?;
            tx.execute("UPDATE provider_clock SET observed_at=max(observed_at,?1) WHERE singleton=1",[now])?;
            super::controller::success(&tx,&config.id,now)?;
            // Completed IDs are retained within the bounded history window; active batches remain pinned.
            prune_history(&tx,now)?;
            let result=get_task(&tx,id)?;tx.commit()?;Ok(result)
        }).await
    }
    pub async fn active_leases(&self) -> Result<Vec<StoredLease>, StorageError> {
        self.call(move|c|Ok(c.prepare("SELECT id,batch_id,provider_id,fingerprint,document,received_at,state FROM provider_leases WHERE state='active' AND expires_at>?1 ORDER BY id DESC LIMIT 100000")?.query_map([unix_now()],lease)?.collect::<Result<Vec<_>,_>>()?)).await
    }
    pub async fn provider_leases(
        &self,
        provider_id: Option<String>,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<LeaseInfo>, StorageError> {
        self.call(move|c| {let now=unix_now();let rows=c.prepare("SELECT id,batch_id,provider_id,fingerprint,document,received_at,state FROM provider_leases WHERE (?1 IS NULL OR provider_id=?1) AND (?2 IS NULL OR id<?2) ORDER BY id DESC LIMIT ?3")?.query_map(params![provider_id,before,limit.clamp(1,200)],lease)?.collect::<Result<Vec<_>,_>>()?;Ok(rows.iter().map(|r|r.info(now)).collect())}).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hps_config::ProviderKind;
    fn config() -> ProviderConfig {
        ProviderConfig {
            id: "source".into(),
            enabled: true,
            kind: ProviderKind::Text,
            endpoint: "http://localhost/list".into(),
            ttl_secs: Some(300),
            ..ProviderConfig::default()
        }
    }
    fn nodes(count: usize) -> FetchResult {
        let c = config();
        let body = (1..=count)
            .map(|n| format!("192.0.2.{n}:8080"))
            .collect::<Vec<_>>()
            .join("\n");
        hps_provider::parse(&c, body.as_bytes(), unix_now()).unwrap()
    }
    async fn next(store: &Store, c: &ProviderConfig, key: &str) -> ExtractionTask {
        store
            .call(|db| {
                db.execute("UPDATE provider_tasks SET started_at=started_at-120", [])?;
                db.execute(
                    "UPDATE provider_control SET next_allowed_at=0,paused_reason=NULL",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let (task, new) = store
            .begin_extraction(c.clone(), key.into(), hps_provider::fingerprint(c))
            .await
            .unwrap();
        assert!(new);
        store.extraction_requesting(task.id).await.unwrap();
        task
    }
    #[tokio::test]
    async fn append_preserves_old_deadline_and_distinct_leases_pagination_and_errors_preserve_pool()
    {
        let store = Store::open(None, DaemonConfig::default()).unwrap();
        let c = config();
        let task = next(&store, &c, "batch-1").await;
        store
            .finish_extraction(task.id, c.clone(), nodes(2), true)
            .await
            .unwrap();
        let old = store
            .provider_leases(Some(c.id.clone()), None, 200)
            .await
            .unwrap();
        assert_eq!(old.len(), 2);
        let deadline = old[0].expires_at;
        let task = next(&store, &c, "batch-2").await;
        store
            .finish_extraction(task.id, c.clone(), nodes(1), true)
            .await
            .unwrap();
        let leases = store
            .provider_leases(Some(c.id.clone()), None, 200)
            .await
            .unwrap();
        assert_eq!(leases.len(), 3);
        assert_ne!(leases[0].id, old[1].id);
        assert_eq!(leases[1].expires_at, deadline);
        let task = next(&store, &c, "empty").await;
        let task = store
            .finish_extraction(
                task.id,
                c.clone(),
                FetchResult {
                    nodes: vec![],
                    rejected: 0,
                    quota: None,
                },
                true,
            )
            .await
            .unwrap();
        assert_eq!(task.state, "failed");
        assert_eq!(store.active_leases().await.unwrap().len(), 3);
        assert_eq!(
            store
                .provider_leases(None, Some(leases[0].id), 1)
                .await
                .unwrap()[0]
                .id,
            leases[1].id
        );
        let text = serde_json::to_string(&leases).unwrap();
        assert!(!text.contains("credentials"));
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn snapshot_shrink_and_stale_source_are_safe_and_inflight_merges() {
        let store = Store::open(None, DaemonConfig::default()).unwrap();
        let mut c = config();
        let task = next(&store, &c, "first").await;
        let (merged, new) = store
            .begin_extraction(
                c.clone(),
                "another-click".into(),
                hps_provider::fingerprint(&c),
            )
            .await
            .unwrap();
        assert_eq!(merged.id, task.id);
        assert!(!new);
        store
            .finish_extraction(task.id, c.clone(), nodes(4), true)
            .await
            .unwrap();
        c.mode = LeaseMode::ReplaceSnapshot;
        c.full_snapshot = true;
        c.minimum_snapshot_percent = 75;
        let task = next(&store, &c, "shrink").await;
        assert_eq!(
            store
                .finish_extraction(task.id, c.clone(), nodes(1), true)
                .await
                .unwrap()
                .state,
            "failed"
        );
        assert_eq!(store.active_leases().await.unwrap().len(), 4);
        let task = next(&store, &c, "stale").await;
        assert_eq!(
            store
                .finish_extraction(task.id, c.clone(), nodes(3), false)
                .await
                .unwrap()
                .state,
            "discarded"
        );
        assert_eq!(store.active_leases().await.unwrap().len(), 4);
        let task = next(&store, &c, "valid-snapshot").await;
        let task = store
            .finish_extraction(task.id, c.clone(), nodes(3), true)
            .await
            .unwrap();
        assert_eq!(task.removed, 4);
        assert_eq!(store.active_leases().await.unwrap().len(), 3);
        let (dedup, new) = store
            .begin_extraction(
                c.clone(),
                "valid-snapshot".into(),
                hps_provider::fingerprint(&c),
            )
            .await
            .unwrap();
        assert!(!new);
        assert_eq!(dedup.id, task.id);
        assert!(matches!(
            store
                .begin_extraction(c.clone(), "too-soon".into(), hps_provider::fingerprint(&c))
                .await,
            Err(StorageError::ProviderRateLimited)
        ));
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn restart_keeps_uncertain_outcome_and_clock_rollback_does_not_extend_leases() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let c = config();
        let task = next(&store, &c, "good").await;
        store
            .finish_extraction(task.id, c.clone(), nodes(1), true)
            .await
            .unwrap();
        let task = next(&store, &c, "lost-response").await;
        store
            .call(|db| {
                db.execute(
                    "UPDATE provider_clock SET observed_at=?1",
                    [unix_now() + 3600],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        store.close().await.unwrap();
        let restored = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let result = restored.extraction_task(task.id).await.unwrap();
        assert_eq!(result.state, "outcome_unknown");
        assert_eq!(result.error_code.as_deref(), Some("process_interrupted"));
        assert!(restored.active_leases().await.unwrap().is_empty());
        assert_eq!(
            restored.provider_leases(None, None, 10).await.unwrap()[0].state,
            "clock_rollback"
        );
        let (dedup, new) = restored
            .begin_extraction(
                c.clone(),
                "lost-response".into(),
                hps_provider::fingerprint(&c),
            )
            .await
            .unwrap();
        assert!(!new);
        assert_eq!(dedup.id, task.id);
        restored.close().await.unwrap();
    }
    #[tokio::test]
    async fn clean_restart_preserves_deadlines_but_unclean_restart_retires_old_leases() {
        let dir = tempfile::tempdir().unwrap();
        let c = config();
        let store = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        let task = next(&store, &c, "clean").await;
        store
            .finish_extraction(task.id, c.clone(), nodes(1), true)
            .await
            .unwrap();
        let expiry = store.active_leases().await.unwrap()[0].candidate.expires_at;
        store.close().await.unwrap();
        let restored = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        assert_eq!(
            restored.active_leases().await.unwrap()[0]
                .candidate
                .expires_at,
            expiry
        );
        restored.close().await.unwrap();
        let fault = Connection::open(dir.path().join("hps.sqlite3")).unwrap();
        fault
            .execute("UPDATE provider_clock SET clean_shutdown=0", [])
            .unwrap();
        drop(fault);
        let restored = Store::open(Some(dir.path()), DaemonConfig::default()).unwrap();
        assert!(restored.active_leases().await.unwrap().is_empty());
        assert_eq!(
            restored.provider_leases(None, None, 10).await.unwrap()[0].state,
            "revoked"
        );
        restored.close().await.unwrap();
    }
    #[tokio::test]
    async fn snapshot_protection_counts_unique_endpoints_instead_of_repeated_leases() {
        let store = Store::open(None, DaemonConfig::default()).unwrap();
        let mut c = config();
        for key in ["a", "b", "c"] {
            let task = next(&store, &c, key).await;
            store
                .finish_extraction(task.id, c.clone(), nodes(1), true)
                .await
                .unwrap();
        }
        assert_eq!(store.active_leases().await.unwrap().len(), 3);
        c.mode = LeaseMode::ReplaceSnapshot;
        c.full_snapshot = true;
        c.minimum_snapshot_percent = 100;
        let task = next(&store, &c, "snapshot").await;
        let result = store
            .finish_extraction(task.id, c, nodes(1), true)
            .await
            .unwrap();
        assert_eq!(result.state, "succeeded");
        assert_eq!(result.removed, 3);
        assert_eq!(store.active_leases().await.unwrap().len(), 1);
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn source_lease_pages_do_not_include_other_provider_history() {
        let store = Store::open(None, DaemonConfig::default()).unwrap();
        let mut a = config();
        a.id = "first-source".into();
        let mut b = config();
        b.id = "second-source".into();
        for c in [&a, &b] {
            let task = next(&store, c, "one").await;
            store
                .finish_extraction(task.id, c.clone(), nodes(1), true)
                .await
                .unwrap();
        }
        let a_rows = store
            .provider_leases(Some(a.id.clone()), None, 50)
            .await
            .unwrap();
        assert_eq!(a_rows.len(), 1);
        assert_eq!(a_rows[0].provider_id, a.id);
        let b_rows = store
            .provider_leases(Some(b.id.clone()), None, 50)
            .await
            .unwrap();
        assert_eq!(b_rows.len(), 1);
        assert_eq!(b_rows[0].provider_id, b.id);
        store.close().await.unwrap();
    }
}
