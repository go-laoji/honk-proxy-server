use super::*;
use axum::{Json, Router, extract::Path as RoutePath, routing::get};
use hps_domain::{HealthTransport, HealthVerdict, IpFamily, NodeQuery, ProbePhase, ProbeResult};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

fn fixture(ip: IpAddr, region: &str) -> (Blob, Vec<u8>) {
    let (family, address, prefix) = match ip {
        IpAddr::V4(ip) => {
            let bytes = ip.octets();
            (
                4,
                bytes.into_iter().rev().collect::<Vec<_>>(),
                [bytes[0], bytes[1]],
            )
        }
        IpAddr::V6(ip) => {
            let bytes = ip.octets();
            (6, bytes.to_vec(), [bytes[0], bytes[1]])
        }
    };
    let first = (INDEX_START + region.len()) as u32;
    let mut bytes = vec![0; INDEX_START];
    bytes[..4].copy_from_slice(&[3, 0, 1, 0]);
    bytes[8..12].copy_from_slice(&first.to_le_bytes());
    bytes[12..16].copy_from_slice(&first.to_le_bytes());
    bytes[16..20].copy_from_slice(&[family, 0, 4, 0]);
    let offset = 256 + u16::from_be_bytes(prefix) as usize * 8;
    bytes[offset..offset + 4].copy_from_slice(&first.to_le_bytes());
    bytes[offset + 4..offset + 8].copy_from_slice(&first.to_le_bytes());
    bytes.extend_from_slice(region.as_bytes());
    bytes.extend_from_slice(&address);
    bytes.extend_from_slice(&address);
    bytes.extend_from_slice(&(region.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(INDEX_START as u32).to_le_bytes());
    let mut sha = Sha1::new();
    sha.update(format!("blob {}\0", bytes.len()));
    sha.update(&bytes);
    (
        Blob {
            sha: format!("{:x}", sha.finalize()),
            size: bytes.len(),
        },
        bytes,
    )
}

#[tokio::test]
async fn updates_both_families_preserve_last_good_and_reuse_observed_exit_ip() {
    let endpoint: IpAddr = "1.2.3.4".parse().unwrap();
    let exit: IpAddr = "240e::1".parse().unwrap();
    let (v4_blob, v4_bytes) = fixture(endpoint, "中国|广东省|广州市|电信|CN");
    let (v6_blob, v6_bytes) = fixture(exit, "中国|江苏省|苏州市|联通|CN");
    let (next_blob, next_bytes) = fixture(exit, "中国|浙江省|杭州市|联通|CN");
    let updating = Arc::new(AtomicBool::new(false));
    let fail = Arc::new(AtomicBool::new(false));
    let reached = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let state = updating.clone();
    let router = Router::new()
        .route(
            "/metadata",
            get(move || {
                let current_v6 = if state.load(Ordering::SeqCst) {
                    &next_blob
                } else {
                    &v6_blob
                };
                let response = serde_json::json!([
                    {"name": V4_FILE, "size": v4_blob.size, "sha": v4_blob.sha},
                    {"name": V6_FILE, "size": current_v6.size, "sha": current_v6.sha},
                ]);
                async { Json(response) }
            }),
        )
        .route(
            "/data/{name}",
            get({
                let updating = updating.clone();
                let fail = fail.clone();
                let reached = reached.clone();
                let release = release.clone();
                move |RoutePath(name): RoutePath<String>| {
                    let in_progress = updating.load(Ordering::SeqCst);
                    let fail = fail.load(Ordering::SeqCst);
                    let reached = reached.clone();
                    let release = release.clone();
                    let bytes = match name.as_str() {
                        V4_FILE => v4_bytes.clone(),
                        V6_FILE if fail => b"partial data".to_vec(),
                        V6_FILE if in_progress => next_bytes.clone(),
                        V6_FILE => v6_bytes.clone(),
                        _ => unreachable!(),
                    };
                    async move {
                        if name == V6_FILE && in_progress && fail {
                            reached.notify_one();
                            release.notified().await;
                        }
                        bytes
                    }
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let root = tempfile::tempdir().unwrap();
    let mut config = hps_config::DaemonConfig::default();
    config.proxy.upstreams.push(hps_config::UpstreamConfig {
        id: "test-node".into(),
        protocol: hps_config::ProxyProtocol::Http,
        host: endpoint.to_string(),
        port: 8080,
        enabled: true,
        expires_at_unix: None,
        credentials: None,
        ca_certificate: None,
        tls_server_name: None,
    });
    config.health.profiles[0].url = format!("http://{address}/check");
    config.health.profiles[0].capture_exit_ip = true;
    let store = hps_storage::Store::open(Some(root.path()), config).unwrap();
    let service = Supervisor::boot(store, Default::default()).await.unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let metadata = format!("http://{address}/metadata");
    let raw = format!("http://{address}/data");
    let first = service
        .update_ip2region_from(client.clone(), &metadata, &raw)
        .await
        .unwrap();
    assert!(first.updated);
    let same = service
        .update_ip2region_from(client.clone(), &metadata, &raw)
        .await
        .unwrap();
    assert!(!same.updated);
    let before = service.node("test-node".into()).await.unwrap();
    let location = before.nodes[0].location.as_ref().unwrap();
    assert_eq!(location.source, NodeLocationSource::EndpointIp);
    assert_eq!(location.city.as_deref(), Some("广州市"));
    let current = service.current();
    let plan = current.engine.probe_plans().remove(0);
    let profile = &current.health_profiles[0];
    current.engine.health_registry().publish(
        ProbeResult {
            node_id: "test-node".into(),
            identity: plan.identity().into(),
            profile_fingerprint: profile.fingerprint.clone(),
            transport: HealthTransport::Tcp,
            family: IpFamily::Ipv4,
            verdict: HealthVerdict::Healthy,
            phase: ProbePhase::Complete,
            error_code: None,
            endpoint_reachable: true,
            authenticated: None,
            tunnel_established: true,
            http_status: Some(200),
            target_ok: Some(true),
            exit_ip: Some(exit),
            latency_ms: 1,
            checked_at: hps_storage::unix_now(),
        },
        current.config.health.max_staleness_secs,
    );
    let found = service
        .nodes(NodeQuery {
            search: Some("苏州".into()),
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(found.filtered_total, 1);
    let location = found.nodes[0].location.as_ref().unwrap();
    assert_eq!(location.source, NodeLocationSource::ExitIp);
    assert_eq!(location.ip, exit);
    assert_eq!(location.city.as_deref(), Some("苏州市"));

    updating.store(true, Ordering::SeqCst);
    fail.store(true, Ordering::SeqCst);
    let attempt = tokio::spawn({
        let (service, client, metadata, raw) = (
            service.clone(),
            client.clone(),
            metadata.clone(),
            raw.clone(),
        );
        async move { service.update_ip2region_from(client, &metadata, &raw).await }
    });
    tokio::time::timeout(Duration::from_secs(3), reached.notified())
        .await
        .unwrap();
    assert!(matches!(
        service
            .update_ip2region_from(client.clone(), &metadata, &raw)
            .await,
        Err(ApplicationError::Ip2RegionBusy)
    ));
    release.notify_one();
    assert!(matches!(
        attempt.await.unwrap(),
        Err(ApplicationError::Ip2RegionUpdate(UpdateError::InvalidData))
    ));
    let unchanged = service.node("test-node".into()).await.unwrap();
    assert_eq!(unchanged.ip2region.unwrap().version, first.database.version);
    assert_eq!(
        unchanged.nodes[0]
            .location
            .as_ref()
            .unwrap()
            .city
            .as_deref(),
        Some("苏州市")
    );

    fail.store(false, Ordering::SeqCst);
    let next = service
        .update_ip2region_from(client, &metadata, &raw)
        .await
        .unwrap();
    assert!(next.updated);
    assert_ne!(next.database.version, first.database.version);
    let reopened = Ip2Region::load(&root.path().join("ip2region"))
        .unwrap()
        .unwrap();
    assert_eq!(reopened.status().version, next.database.version);
    assert_eq!(
        reopened
            .locate(exit, NodeLocationSource::ExitIp)
            .unwrap()
            .city
            .as_deref(),
        Some("杭州市")
    );
    service.shutdown().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn extracted_leases_follow_provider_region_rules_and_database_updates() {
    use hps_config::{
        GroupConfig, ProviderConfig, ProviderKind, ProviderRegionRule, SelectionSave,
    };
    use hps_domain::{ExtractionInput, SelectionQuery};

    let ipv4 = "1.2.3.4".parse().unwrap();
    let ipv6 = "240e::1".parse().unwrap();
    let (v4, v4_bytes) = fixture(ipv4, "中国|广东省|广州市|电信|CN");
    let (v6, v6_bytes) = fixture(ipv6, "中国|江苏省|苏州市|联通|CN");
    let (next_v4, next_v4_bytes) = fixture(ipv4, "中国|浙江省|杭州市|电信|CN");
    let metadata = serde_json::json!([
        {"name": V4_FILE, "size": next_v4.size, "sha": next_v4.sha},
        {"name": V6_FILE, "size": v6.size, "sha": v6.sha},
    ]);
    let initial = Manifest { ipv4: v4, ipv6: v6 };
    let next_v6_bytes = v6_bytes.clone();
    let router = Router::new()
        .route(
            "/extract",
            get(|| async { "1.2.3.4:8080\n[240e::1]:8080\n127.0.0.1:8080\n" }),
        )
        .route(
            "/metadata",
            get(move || {
                let metadata = metadata.clone();
                async { Json(metadata) }
            }),
        )
        .route(
            "/data/{name}",
            get(move |RoutePath(name): RoutePath<String>| {
                let bytes = if name == V4_FILE {
                    next_v4_bytes.clone()
                } else {
                    next_v6_bytes.clone()
                };
                async { bytes }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut config = hps_config::DaemonConfig::default();
    config.health.enabled = false;
    config.providers.push(ProviderConfig {
        id: "vendor".into(),
        enabled: true,
        kind: ProviderKind::Text,
        endpoint: format!("http://{address}/extract"),
        count: 3,
        ttl_secs: Some(300),
        ..Default::default()
    });
    let group =
        |id: &str, isp: Option<&str>, province: Option<&str>, city: Option<&str>| GroupConfig {
            id: id.into(),
            provider_regions: vec![ProviderRegionRule {
                provider: "vendor".into(),
                isp: isp.map(str::to_owned),
                province: province.map(str::to_owned),
                city: city.map(str::to_owned),
            }],
            ..Default::default()
        };
    config.proxy.selection.groups = vec![
        group("guangzhou", Some("中国电信"), Some("广东"), Some("广州")),
        group("unicom", Some("联通"), None, None),
        GroupConfig {
            id: "combined".into(),
            groups: vec!["guangzhou".into(), "unicom".into()],
            ..Default::default()
        },
    ];
    let store = hps_storage::Store::open(None, config).unwrap();
    let service = Supervisor::boot(store, Default::default()).await.unwrap();
    service.confirm_startup().await.unwrap();
    service
        .ip2region
        .install(initial, &v4_bytes, &v6_bytes, Ip2RegionSource::Updated)
        .unwrap();
    let task = service
        .extract(
            "vendor".into(),
            ExtractionInput {
                request_id: "test-provider-regions".into(),
            },
        )
        .await
        .unwrap();
    let decision = |group: &str| {
        service
            .simulate_group(SelectionQuery {
                group: Some(group.into()),
                ..Default::default()
            })
            .unwrap()
    };
    tokio::time::timeout(Duration::from_secs(3), async {
        while decision("combined").eligible != 2 {
            assert_ne!(
                service.extraction_task(task.id).await.unwrap().state,
                "failed"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let inventory = service
        .nodes(NodeQuery {
            group: Some("guangzhou".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(inventory.filtered_total, 1);
    assert_eq!(inventory.nodes[0].host, "1.2.3.4");
    assert_eq!(
        decision("guangzhou").selected.as_deref(),
        Some(inventory.nodes[0].id.as_str())
    );
    let leases = service
        .provider_leases(Some("vendor".into()), None, 50)
        .await
        .unwrap();
    assert_eq!(leases.len(), 3);
    assert!(
        leases
            .iter()
            .find(|lease| lease.host == "127.0.0.1")
            .unwrap()
            .location
            .is_none()
    );
    assert_eq!(
        leases
            .iter()
            .find(|lease| lease.host == "240e::1")
            .unwrap()
            .location
            .as_ref()
            .unwrap()
            .city
            .as_deref(),
        Some("苏州市")
    );

    let before = service.groups_overview();
    let mut configuration = before.configuration;
    configuration
        .groups
        .push(group("hangzhou", None, None, Some("杭州")));
    configuration.groups[1].provider_regions[0].province = Some("江苏省".into());
    let activated = service
        .edit_groups(SelectionSave {
            configuration,
            expected_revision: before.running_revision,
        })
        .await
        .unwrap();
    assert_eq!(activated.state, "applied");
    assert_eq!(decision("unicom").eligible, 1);
    assert_eq!(decision("hangzhou").eligible, 0);

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let update = service
        .update_ip2region_from(
            client,
            &format!("http://{address}/metadata"),
            &format!("http://{address}/data"),
        )
        .await
        .unwrap();
    assert!(update.updated);
    assert_eq!(decision("guangzhou").eligible, 0);
    assert_eq!(decision("hangzhou").eligible, 1);
    assert_eq!(decision("combined").eligible, 1);
    let moved = service
        .nodes(NodeQuery {
            group: Some("hangzhou".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(moved.nodes[0].id, inventory.nodes[0].id);
    assert_eq!(service.extraction_task(task.id).await.unwrap().added, 3);
    service.shutdown().await.unwrap();
    server.abort();
}
