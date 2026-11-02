use crate::ProxyError;
use hps_config::{ServerTls, UpstreamConfig};
use std::{
    io::BufReader,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore, ServerConfig};
use x509_parser::prelude::FromDer;

#[derive(Debug, Clone)]
pub struct CertificateInfo {
    pub not_before_unix: i64,
    pub not_after_unix: i64,
    pub valid_now: bool,
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn server(config: &ServerTls) -> Result<(Arc<ServerConfig>, CertificateInfo), ProxyError> {
    let bytes = std::fs::read(&config.certificate).map_err(|_| ProxyError::TlsConfig)?;
    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(bytes.as_slice()))
        .collect::<Result<_, _>>()
        .map_err(|_| ProxyError::TlsConfig)?;
    let first = certs.first().ok_or(ProxyError::TlsConfig)?;
    let (_, cert) = x509_parser::certificate::X509Certificate::from_der(first.as_ref())
        .map_err(|_| ProxyError::TlsConfig)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let info = CertificateInfo {
        not_before_unix: cert.validity().not_before.timestamp(),
        not_after_unix: cert.validity().not_after.timestamp(),
        valid_now: now >= cert.validity().not_before.timestamp()
            && now <= cert.validity().not_after.timestamp(),
    };
    let key = std::fs::read(&config.private_key).map_err(|_| ProxyError::TlsConfig)?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(key.as_slice()))
        .map_err(|_| ProxyError::TlsConfig)?
        .ok_or(ProxyError::TlsConfig)?;
    let mut server = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|_| ProxyError::TlsConfig)?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|_| ProxyError::TlsConfig)?;
    server.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok((Arc::new(server), info))
}

pub fn client(config: &UpstreamConfig) -> Result<Arc<ClientConfig>, ProxyError> {
    let mut roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = &config.ca_certificate {
        let bytes = std::fs::read(path).map_err(|_| ProxyError::TlsConfig)?;
        let mut reader = BufReader::new(bytes.as_slice());
        let certs = rustls_pemfile::certs(&mut reader);
        let mut count = 0;
        for cert in certs {
            roots
                .add(cert.map_err(|_| ProxyError::TlsConfig)?)
                .map_err(|_| ProxyError::TlsConfig)?;
            count += 1;
        }
        if count == 0 {
            return Err(ProxyError::TlsConfig);
        }
    }
    let mut client = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|_| ProxyError::TlsConfig)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    client.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(client))
}
