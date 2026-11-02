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
