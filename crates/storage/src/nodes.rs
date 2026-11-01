use super::*;
use hps_domain::{NodeControl, NodeOperationState};

/// The lease identity viewed by the caller, never a provider-wide deletion selector.
#[derive(Debug, Clone)]
pub struct LeaseScope {
    pub id: i64,
    pub batch_id: i64,
    pub provider_id: String,
}
fn control(row: &rusqlite::Row<'_>) -> rusqlite::Result<NodeControl> {
    let state: String = row.get(1)?;
    Ok(NodeControl {
        node_id: row.get(0)?,
        state: match state.as_str() {
            "enabled" => NodeOperationState::Enabled,
            "disabled" => NodeOperationState::Disabled,
            "draining" => NodeOperationState::Draining,
            "deleted" => NodeOperationState::Deleted,
            _ => return Err(rusqlite::Error::InvalidQuery),
        },
        version: row.get(2)?,
        updated_at: row.get(3)?,
    })
}
impl Store {
    pub async fn node_controls(&self) -> Result<Vec<NodeControl>, StorageError> {
        self.call(|c| Ok(c.prepare("SELECT node_id,state,version,updated_at FROM node_controls ORDER BY node_id LIMIT 300000")?.query_map([],control)?.collect::<Result<Vec<_>,_>>()?)).await
    }
    pub async fn set_node_control(
        &self,
        node_id: String,
        state: NodeOperationState,
        expected_version: i64,
        expected_revision: i64,
        lease: Option<LeaseScope>,
    ) -> Result<NodeControl, StorageError> {
        if !hps_config::valid_node_id(&node_id) || expected_version < 0 {
            return Err(StorageError::Invalid);
        }
        self.call(move |c| {
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            check_head(&tx, expected_revision)?;
            if let Some(scope) = lease {
                if node_id != format!("lease:{}",scope.id) { return Err(StorageError::Invalid); }
                let matches: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_leases WHERE id=?1 AND batch_id=?2 AND provider_id=?3)", params![scope.id,scope.batch_id,scope.provider_id], |r| r.get(0))?;
                if !matches { return Err(StorageError::NotFound); }
            } else if node_id.starts_with("lease:") {
                return Err(StorageError::Invalid);
            }
            let previous: Option<i64> = tx.query_row("SELECT version FROM node_controls WHERE node_id=?1", [&node_id], |r|r.get(0)).optional()?;
            if previous.unwrap_or(0) != expected_version { return Err(StorageError::Conflict); }
            // Lease histories are bounded independently; discard restrictions only after the
            // corresponding immutable lease record has actually been pruned.
            tx.execute("DELETE FROM node_controls WHERE node_id LIKE 'lease:%' AND NOT EXISTS(SELECT 1 FROM provider_leases WHERE 'lease:' || provider_leases.id=node_controls.node_id)", [])?;
            if previous.is_none() {
                let count: i64 = tx.query_row("SELECT count(*) FROM node_controls",[],|r|r.get(0))?;
                if count >= 300000 { return Err(StorageError::PoolFull); }
            }
            let version = expected_version.checked_add(1).ok_or(StorageError::Invalid)?;
            let updated_at = unix_now();
            tx.execute("INSERT INTO node_controls(node_id,state,version,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(node_id) DO UPDATE SET state=excluded.state,version=excluded.version,updated_at=excluded.updated_at", params![node_id,state.as_str(),version,updated_at])?;
            tx.commit()?;
            Ok(NodeControl { node_id,state,version,updated_at })
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn lease_control_requires_exact_batch_and_preserves_other_lease_and_original_deadlines() {
        let store = Store::open(None, DaemonConfig::default()).unwrap();
        let config = hps_config::ProviderConfig {
            id: "source".into(),
            enabled: true,
            kind: hps_config::ProviderKind::Text,
            endpoint: "http://localhost/list".into(),
            ttl_secs: Some(300),
            ..Default::default()
        };
        let (task, _) = store
            .begin_extraction(
                config.clone(),
                "first".into(),
                hps_provider::fingerprint(&config),
            )
            .await
            .unwrap();
        store.extraction_requesting(task.id).await.unwrap();
        let result =
            hps_provider::parse(&config, b"192.0.2.1:8080\n192.0.2.2:8080", unix_now()).unwrap();
        store
            .finish_extraction(task.id, config, result, true)
            .await
            .unwrap();
        let before = store.provider_leases(None, None, 200).await.unwrap();
        let chosen = &before[0];
        let scope = LeaseScope {
            id: chosen.id,
            batch_id: chosen.batch_id,
            provider_id: chosen.provider_id.clone(),
        };
        let mut wrong = scope.clone();
        wrong.batch_id += 1;
        assert!(matches!(
            store
                .set_node_control(
                    chosen.upstream_id.clone(),
                    NodeOperationState::Deleted,
                    0,
                    1,
                    Some(wrong)
                )
                .await,
            Err(StorageError::NotFound)
        ));
        store
            .set_node_control(
                chosen.upstream_id.clone(),
                NodeOperationState::Deleted,
                0,
                1,
                Some(scope),
            )
            .await
            .unwrap();
        let after = store.provider_leases(None, None, 200).await.unwrap();
        assert_eq!(after.len(), before.len());
        for (a, b) in after.iter().zip(before) {
            assert_eq!(a.expires_at, b.expires_at);
            assert_eq!(a.state, b.state);
        }
        assert_eq!(store.node_controls().await.unwrap().len(), 1);
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn operational_state_survives_restart_and_rejects_stale_writes() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(Some(directory.path()), DaemonConfig::default()).unwrap();
        let initial = store
            .set_node_control("manual".into(), NodeOperationState::Draining, 0, 1, None)
            .await
            .unwrap();
        assert_eq!(initial.version, 1);
        assert!(matches!(
            store
                .set_node_control("manual".into(), NodeOperationState::Enabled, 0, 1, None)
                .await,
            Err(StorageError::Conflict)
        ));
        assert!(matches!(
            store
                .set_node_control("manual".into(), NodeOperationState::Enabled, 1, 2, None)
                .await,
            Err(StorageError::Conflict)
        ));
        store.close().await.unwrap();
        let store = Store::open(Some(directory.path()), DaemonConfig::default()).unwrap();
        let controls = store.node_controls().await.unwrap();
        assert_eq!(controls[0].state, NodeOperationState::Draining);
        assert_eq!(
            store
                .set_node_control("manual".into(), NodeOperationState::Enabled, 1, 1, None)
                .await
                .unwrap()
                .version,
            2
        );
        assert!(
            store
                .set_node_control("lease:1".into(), NodeOperationState::Deleted, 0, 1, None)
                .await
                .is_err()
        );
        store.close().await.unwrap();
    }
}
