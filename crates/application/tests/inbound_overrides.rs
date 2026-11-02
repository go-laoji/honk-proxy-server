#[path = "../../engine/tests/support/mod.rs"]
mod support;
use hps_application::{StartupOverrides, Supervisor};
use hps_config::{DaemonConfig, ProxyProtocol, RoutingConfig, RulesSave};
use hps_domain::{ConfigActivate, ConfigInput, RouteAction};
use hps_storage::Store;
use std::sync::Arc;
async fn apply_config(service: &Arc<Supervisor>, config: &DaemonConfig) {
    let expected = service.current().revision;
    let revision = service
        .create_revision(ConfigInput {
            toml: toml::to_string(config).unwrap(),
            expected_revision: expected,
            expected_draft_version: None,
            note: "explicit enabled intent".into(),
            use_draft_credentials: false,
        })
        .await
        .unwrap();
    service
        .activate(ConfigActivate {
            revision: revision.id,
            expected_revision: expected,
            defer_until_restart: false,
        })
        .await
        .unwrap();
}
async fn apply_rule(service: &Arc<Supervisor>, action: RouteAction) {
    let expected = service.current().revision;
    let revision = service
        .save_rules(RulesSave {
            configuration: RoutingConfig {
                default_action: action,
                ..Default::default()
            },
            expected_revision: expected,
            base_revision: None,
        })
        .await
        .unwrap();
    service
        .apply_rules(ConfigActivate {
            revision: revision.id,
            expected_revision: expected,
            defer_until_restart: false,
        })
        .await
        .unwrap();
}
#[tokio::test]
async fn manual_inbound_overrides_survive_hot_rules_until_enabled_changes_or_daemon_restarts() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    let mut inbound = support::inbound("http", ProxyProtocol::Http, None);
    inbound.enabled = false;
    config.proxy.inbounds = vec![inbound];
    let service = Supervisor::boot(
        Store::open(Some(directory.path()), config).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    service.confirm_startup().await.unwrap();
    let started = service.start_inbound("http").await.unwrap();
    let address = started.listen;
    assert_eq!(started.runtime_override, Some(true));
    assert!(!started.configured_enabled);
    apply_rule(&service, RouteAction::Direct).await;
    let running = service.current().engine.status().inbounds.remove(0);
    assert!(running.running);
    assert_eq!(running.listen, address);
    assert_eq!(running.runtime_override, Some(true));
    let mut config = service.current().config.clone();
    config.proxy.inbounds[0].enabled = true;
    apply_config(&service, &config).await;
    let running = service.current().engine.status().inbounds.remove(0);
    assert!(running.running);
    assert_eq!(running.runtime_override, None);
    service.stop_inbound("http").await.unwrap();
    apply_rule(&service, RouteAction::Reject).await;
    let stopped = service.current().engine.status().inbounds.remove(0);
    assert!(!stopped.running);
    assert!(stopped.configured_enabled);
    assert_eq!(stopped.runtime_override, Some(false));
    let mut config = service.current().config.clone();
    config.proxy.inbounds[0].enabled = false;
    apply_config(&service, &config).await;
    assert_eq!(
        service.current().engine.status().inbounds[0].runtime_override,
        None
    );
    config.proxy.inbounds[0].enabled = true;
    apply_config(&service, &config).await;
    assert!(service.current().engine.status().inbounds[0].running);
    service.stop_inbound("http").await.unwrap();
    service.shutdown().await.unwrap();
    drop(service);
    let reopened = Supervisor::boot(
        Store::open(Some(directory.path()), DaemonConfig::default()).unwrap(),
        StartupOverrides::default(),
    )
    .await
    .unwrap();
    let running = &reopened.current().engine.status().inbounds[0];
    assert!(running.running);
    assert!(running.configured_enabled);
    assert_eq!(running.runtime_override, None);
    reopened.shutdown().await.unwrap();
}
