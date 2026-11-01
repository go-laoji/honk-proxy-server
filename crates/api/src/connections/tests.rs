use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

async fn tunnel(application: &Arc<Supervisor>, target: std::net::SocketAddr) -> TcpStream {
    let mut socket = TcpStream::connect(
        application.current().engine.status().inbounds[0]
            .listen
            .clone()
            .unwrap(),
    )
    .await
    .unwrap();
    socket
        .write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut head = vec![];
    while !head.ends_with(b"\r\n\r\n") {
        head.push(socket.read_u8().await.unwrap());
    }
    assert!(head.starts_with(b"HTTP/1.1 200"));
    socket
}
#[tokio::test]
async fn actual_http_typed_clients_review_concrete_ids_keep_new_flows_and_return_recovery_codes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = listener.local_addr().unwrap();
    let origin = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut body = vec![];
                if socket.read_to_end(&mut body).await.is_ok() {
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                }
            });
        }
    });
    let mut config = DaemonConfig::default();
    config.health.enabled = false;
    config.proxy.routing.default_action = hps_domain::RouteAction::Direct;
    config.proxy.inbounds.push(hps_config::InboundConfig {
        id: "http".into(),
        protocol: hps_config::ProxyProtocol::Http,
        listen: "127.0.0.1:0".parse().unwrap(),
        enabled: true,
        tls: None,
        max_connections: 32,
        bytes_per_second: 0,
    });
    let application = Supervisor::boot(
        hps_storage::Store::open(None, config).unwrap(),
        hps_application::StartupOverrides::default(),
    )
    .await
    .unwrap();
    application.confirm_startup().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(AppState::with_application(application.clone()));
    let server = tokio::spawn(async move {
        axum::serve(listener, crate::router(state)).await.unwrap();
    });
    let client = hps_client::HpsClient::new(
        &format!("http://{address}"),
        std::time::Duration::from_secs(3),
    )
    .unwrap();
    let mut first = tunnel(&application, target).await;
    let port = first.local_addr().unwrap().port();
    let page = client
        .connections(&ConnectionQuery::default())
        .await
        .unwrap();
    assert_eq!(page.filtered_total, 1);
    assert_eq!(page.records[0].context.source_port, Some(port));
    let mut later = tunnel(&application, target).await;
    let preview = client
        .preview_disconnect_connections(&ConnectionDisconnectPreviewInput {
            snapshot: page.snapshot.clone(),
            expected_epoch: page.epoch.clone(),
            ids: vec![],
        })
        .await
        .unwrap();
    assert_eq!(preview.targets.len(), 1);
    let summary = client.connection_summary().await.unwrap();
    assert_eq!(summary.statistics.active, 2);
    assert_eq!(summary.proxy.active_connections, 2);
    for _ in 0..4 {
        client
            .connections(&ConnectionQuery::default())
            .await
            .unwrap();
    }
    assert!(
        matches!(client.connections(&ConnectionQuery {snapshot:Some(page.snapshot.clone()),..Default::default()}).await,Err(hps_client::ClientError::Api {status:409,error})if error.code=="connection_snapshot_expired")
    );
    let input = ConnectionDisconnectConfirm {
        token: preview.token,
        expected_epoch: page.epoch.clone(),
    };
    let confirmed = client.confirm_disconnect_connections(&input).await.unwrap();
    assert_eq!(confirmed.requested, 1);
    assert_eq!(
        client
            .confirm_disconnect_connections(&input)
            .await
            .unwrap()
            .requested_at_unix_ms,
        confirmed.requested_at_unix_ms
    );
    let mut body = vec![];
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        first.read_to_end(&mut body),
    )
    .await
    .unwrap();
    later.write_all(b"only-reviewed-flow-ended").await.unwrap();
    later.shutdown().await.unwrap();
    body.clear();
    later.read_to_end(&mut body).await.unwrap();
    assert_eq!(body, b"only-reviewed-flow-ended");
    let detail = client.connection(&page.records[0].id).await.unwrap();
    assert_eq!(detail.state, "disconnected");
    assert!(detail.ended_at_unix_ms.is_some());
    assert!(
        matches!(client.disconnect_connection(&ConnectionDisconnectInput {id:detail.id.clone(),expected_epoch:uuid::Uuid::new_v4().to_string()}).await,Err(hps_client::ClientError::Api {status:409,error})if error.code=="connection_instance_changed")
    );
    let old = client
        .disconnect_connection(&ConnectionDisconnectInput {
            id: detail.id,
            expected_epoch: page.epoch,
        })
        .await
        .unwrap();
    assert_eq!(old.already_ended, 1);
    assert!(matches!(
        client
            .connections(&ConnectionQuery {
                limit: Some(201),
                ..Default::default()
            })
            .await,
        Err(hps_client::ClientError::Api { status: 422, .. })
    ));
    server.abort();
    application.shutdown().await.unwrap();
    origin.abort();
}
