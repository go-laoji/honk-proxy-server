//! Management server shared by the headless command and Desktop.

use hps_application::Supervisor;
use std::{future::Future, net::SocketAddr, sync::Arc};

mod management;
use management::ManagementListener;

pub type ServerError = Box<dyn std::error::Error + Send + Sync>;

pub struct ManagementServer {
    application: Arc<Supervisor>,
    listener: ManagementListener,
    address: SocketAddr,
}

impl ManagementServer {
    pub async fn bind(application: Arc<Supervisor>) -> Result<Self, ServerError> {
        let current = application.current();
        let result = async {
            let listener = ManagementListener::bind(
                current.config.management_listen,
                current.management_tls.clone(),
            )
            .await?;
            let address = listener.address()?;
            application.confirm_startup().await?;
            Ok::<_, ServerError>((listener, address))
        }
        .await;
        match result {
            Ok((listener, address)) => Ok(Self {
                application,
                listener,
                address,
            }),
            Err(error) => {
                let _ = application.shutdown().await;
                Err(error)
            }
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub async fn serve(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), ServerError> {
        let Self {
            application,
            listener,
            ..
        } = self;
        let stopping = application.clone();
        let (shutdown_done, shutdown_result) = tokio::sync::oneshot::channel();
        let result = axum::serve(
            axum::serve::ListenerExt::tap_io(listener, |_| {}),
            hps_api::router(Arc::new(hps_api::AppState::with_application(
                application.clone(),
            )))
            .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            shutdown.await;
            let _ = shutdown_done.send(stopping.shutdown().await);
        })
        .await;
        if let Ok(result) = shutdown_result.await {
            result?;
        } else {
            application.shutdown().await?;
        }
        result?;
        Ok(())
    }
}
