use hps_engine::{BoxIo, tls::PreparedServer};
use std::{io, net::SocketAddr, time::Duration};
use tokio::{net::TcpListener, task::JoinSet};
use tokio_rustls::TlsAcceptor;

pub struct ManagementListener {
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
    pending: JoinSet<Option<(BoxIo, SocketAddr)>>,
}
impl ManagementListener {
    pub async fn bind(address: SocketAddr, tls: Option<PreparedServer>) -> io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(address).await?,
            tls: tls.map(|tls| TlsAcceptor::from(tls.config)),
            pending: JoinSet::new(),
        })
    }
    pub fn address(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}
impl axum::serve::Listener for ManagementListener {
    type Io = BoxIo;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            tokio::select! {
                accepted=self.listener.accept(),if self.pending.len()<64=>{
                    match accepted {
                        Ok((tcp,address))=>{
                            if let Some(tls)=self.tls.clone(){
                                self.pending.spawn(async move{match tokio::time::timeout(Duration::from_secs(10),tls.accept(tcp)).await{Ok(Ok(tls))=>Some((Box::new(tls) as BoxIo,address)),_=>None}});
                            }else{return (Box::new(tcp),address);}
                        }
                        Err(error)=>{tracing::warn!(%error,"management accept failed");tokio::time::sleep(Duration::from_millis(100)).await;}
                    }
                }
                completed=self.pending.join_next(),if !self.pending.is_empty()=>{if let Some(Ok(Some(connection)))=completed{return connection;}}
            }
        }
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn management_https_verifies_certificate_and_slow_handshakes_do_not_block_others() {
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let cert = directory.path().join("cert.pem");
        let key = directory.path().join("key.pem");
        std::fs::write(&cert, certificate.cert.pem()).unwrap();
        std::fs::write(&key, certificate.signing_key.serialize_pem()).unwrap();
        let prepared = hps_engine::tls::prepare_server(&hps_config::ServerTls {
            certificate: cert.clone(),
            private_key: key,
        })
        .unwrap();
        let listener = ManagementListener::bind("127.0.0.1:0".parse().unwrap(), Some(prepared))
            .await
            .unwrap();
        let address = listener.address().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route(
                    "/healthz",
                    axum::routing::get(|| async { axum::Json(serde_json::json!({"alive":true})) }),
                ),
            )
            .await
            .unwrap();
        });
        let _slow = tokio::net::TcpStream::connect(address).await.unwrap();
        let url = format!("https://localhost:{}/healthz", address.port());
        let trusted = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .add_root_certificate(
                reqwest::Certificate::from_pem(&std::fs::read(cert).unwrap()).unwrap(),
            )
            .build()
            .unwrap();
        assert_eq!(
            trusted
                .get(&url)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap(),
            serde_json::json!({"alive":true})
        );
        let untrusted = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert!(untrusted.get(&url).send().await.is_err());
        let typed = hps_client::HpsClient::with_ca(
            &format!("https://localhost:{}", address.port()),
            Duration::from_secs(2),
            Some(certificate.cert.pem().as_bytes()),
        )
        .unwrap();
        assert_eq!(
            typed
                .get_json::<serde_json::Value>("healthz")
                .await
                .unwrap(),
            serde_json::json!({"alive":true})
        );
        task.abort();
    }
}
