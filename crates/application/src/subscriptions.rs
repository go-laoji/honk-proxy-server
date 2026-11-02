use super::*;
use hps_config::{
    SubscriptionConfig, SubscriptionRefresh, SubscriptionSnapshot, SubscriptionsOverview,
};
impl Supervisor {
    pub fn subscriptions(&self) -> SubscriptionsOverview {
        let current = self.current();
        SubscriptionsOverview {running_revision:current.revision,subscriptions:current.config.subscriptions.iter().map(SubscriptionConfig::redacted).collect(),expiry_semantics:"Snapshot receive time is not supplier expiry. Standard subscription expiry is unknown; short-lived extraction leases remain separate.".into()}
    }
    async fn publish_subscriptions(
        self: &Arc<Self>,
        config: DaemonConfig,
        expected: i64,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let revision = self
            .create_revision(ConfigInput {
                toml: toml::to_string(&config).map_err(|_| ApplicationError::Invalid)?,
                expected_revision: expected,
                expected_draft_version: None,
                note: "Standard subscription snapshot/policy".into(),
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
    pub async fn edit_subscription(
        self: &Arc<Self>,
        id: String,
        input: Option<SubscriptionConfig>,
        expected: i64,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let current = self.current();
        let workspace = self.store.workspace().await?;
        if current.revision != expected || workspace.desired.revision.id != expected {
            return Err(StorageError::Conflict.into());
        }
        let mut config = current.base_config.clone();
        if let Some(mut incoming) = input {
            if incoming.id != id {
                return Err(ApplicationError::Invalid);
            }
            let old = config.subscriptions.iter().find(|s| s.id == id);
            if incoming.url == "[REDACTED]" {
                incoming.url = old.ok_or(ApplicationError::Invalid)?.url.clone();
            }
            // Definition edits cannot inject or mutate cache credentials. Fetch/import owns snapshots.
            incoming.snapshot = old
                .filter(|old| {
                    hps_provider::subscription::fingerprint(old)
                        == hps_provider::subscription::fingerprint(&incoming)
                })
                .and_then(|s| s.snapshot.clone());
            config.subscriptions.retain(|s| s.id != id);
            config.subscriptions.push(incoming);
        } else {
            let before = config.subscriptions.len();
            config.subscriptions.retain(|s| s.id != id);
            if before == config.subscriptions.len() {
                return Err(StorageError::NotFound.into());
            }
        }
        self.publish_subscriptions(config, expected).await
    }
    pub async fn refresh_subscription(
        self: &Arc<Self>,
        id: String,
        input: SubscriptionRefresh,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let permit = self
            .provider_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::ExtractionCapacity)?;
        self.tracked(move |service| async move {
            let _permit = permit;
            let current = service.current();
            if current.revision != input.expected_revision {
                return Err(StorageError::Conflict.into());
            }
            let source = current
                .base_config
                .subscriptions
                .iter()
                .find(|s| s.id == id && s.enabled)
                .ok_or(StorageError::NotFound)?;
            let origin = if input.document.is_some() {
                "document_import"
            } else {
                "remote_get"
            };
            let nodes = if let Some(document) = input.document {
                let source = source.clone();
                tokio::task::spawn_blocking(move || {
                    hps_provider::subscription::parse(&source, &document)
                })
                .await
                .map_err(|_| ApplicationError::Worker)??
            } else {
                hps_provider::subscription::fetch(source, service.name_resolver()).await?
            };
            // Recheck CAS after network/parse work. Failed or stale refresh never clears a pool.
            if service.current().revision != input.expected_revision {
                return Err(StorageError::Conflict.into());
            }
            let mut config = current.base_config.clone();
            config
                .subscriptions
                .iter_mut()
                .find(|s| s.id == id)
                .expect("checked subscription")
                .snapshot = Some(SubscriptionSnapshot {
                fetched_at_unix: hps_storage::unix_now(),
                origin: origin.into(),
                nodes,
            });
            service
                .publish_subscriptions(config, input.expected_revision)
                .await
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn prepared_generation_owns_subscription_snapshot_before_publication_and_failure_keeps_old()
     {
        let mut config = DaemonConfig::default();
        config.health.enabled = false;
        config.subscriptions.push(SubscriptionConfig {
            id: "fixture".into(),
            url: "https://example.test/sub".into(),
            ..Default::default()
        });
        let store = Store::open(None, config).unwrap();
        let service = Supervisor::boot(store.clone(), Default::default())
            .await
            .unwrap();
        let revision = service.subscriptions().running_revision;
        service
            .refresh_subscription(
                "fixture".into(),
                SubscriptionRefresh {
                    expected_revision: revision,
                    document: Some("http://127.0.0.1:8080".into()),
                },
            )
            .await
            .unwrap();
        let old = service.current();
        let old_id = old.engine.selection(&Default::default()).selected.unwrap();
        let mut candidate = old.base_config.clone();
        let source = &mut candidate.subscriptions[0];
        let nodes = hps_provider::subscription::parse(source, "http://127.0.0.1:8081").unwrap();
        let new_id = nodes[0].id.clone();
        source.snapshot = Some(SubscriptionSnapshot {
            fetched_at_unix: hps_storage::unix_now(),
            origin: "document_import".into(),
            nodes,
        });
        // Same source definition fingerprint, changed contents: the precise case
        // where sharing the old DynamicPool previously exposed retired nodes.
        assert_eq!(
            hps_provider::subscription::fingerprint(&candidate.subscriptions[0]),
            hps_provider::subscription::fingerprint(&old.config.subscriptions[0])
        );
        let before = old.clone();
        let preparing = candidate.clone();
        let prepared =
            tokio::task::spawn_blocking(move || prepare_engine(&preparing, Some(&before)))
                .await
                .unwrap()
                .unwrap();
        assert!(Arc::ptr_eq(
            &old.engine.dynamic_pool(),
            &prepared.engine.dynamic_pool()
        ));
        assert!(prepared.engine.prepare_subscription_nodes(vec![]).is_err());
        // No refresh_pool or start/publication has occurred. New readers already
        // see exactly the prepared snapshot; old generation readers retain theirs.
        assert_eq!(
            prepared
                .engine
                .selection(&Default::default())
                .selected
                .as_deref(),
            Some(new_id.as_str())
        );
        assert_eq!(
            old.engine
                .selection(&Default::default())
                .selected
                .as_deref(),
            Some(old_id.as_str())
        );
        assert_eq!(service.current().revision, old.revision);
        let mut failing = candidate.clone();
        let node = &mut failing.subscriptions[0].snapshot.as_mut().unwrap().nodes[0];
        node.protocol = hps_config::ProxyProtocol::Https;
        node.ca_certificate = Some("/nonexistent/hps-test-subscription-ca.pem".into());
        let before = old.clone();
        assert!(
            tokio::task::spawn_blocking(move || prepare_engine(&failing, Some(&before)))
                .await
                .unwrap()
                .is_err()
        );
        assert_eq!(
            service
                .current()
                .engine
                .selection(&Default::default())
                .selected
                .as_deref(),
            Some(old_id.as_str())
        );
        assert_eq!(
            store.workspace().await.unwrap().desired.revision.id,
            old.revision
        );
        let applied = service
            .refresh_subscription(
                "fixture".into(),
                SubscriptionRefresh {
                    expected_revision: old.revision,
                    document: Some("http://127.0.0.1:8081".into()),
                },
            )
            .await
            .unwrap();
        assert!(applied.running_revision > old.revision);
        assert_eq!(
            service
                .current()
                .engine
                .selection(&Default::default())
                .selected
                .as_deref(),
            Some(new_id.as_str())
        );
        assert_eq!(
            old.engine
                .selection(&Default::default())
                .selected
                .as_deref(),
            Some(old_id.as_str())
        );
        service.shutdown().await.unwrap();
    }
}
