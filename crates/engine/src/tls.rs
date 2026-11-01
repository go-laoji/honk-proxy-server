use crate::ProxyError;
use hps_config::{ServerTls, UpstreamConfig};
use sha2::{Digest, Sha256};
use std::{
    io::BufReader,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_rustls::rustls::{self, ClientConfig, RootCertStore, ServerConfig};
use x509_parser::prelude::FromDer;

fn read_pem(path: &std::path::Path) -> Result<Vec<u8>, ProxyError> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| ProxyError::TlsConfig)?;
    let metadata = file.metadata().map_err(|_| ProxyError::TlsConfig)?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(ProxyError::TlsConfig);
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ProxyError::TlsConfig)?;
    if bytes.len() > 1024 * 1024 {
        return Err(ProxyError::TlsConfig);
    }
    Ok(bytes)
}

#[derive(Debug, Clone)]
pub struct CertificateInfo {
    pub not_before_unix: i64,
    pub not_after_unix: i64,
    pub valid_now: bool,
    pub fingerprint: String,
}
#[derive(Clone)]
pub struct PreparedServer {
    pub config: Arc<ServerConfig>,
    pub certificate: CertificateInfo,
}
pub fn prepare_server(config: &ServerTls) -> Result<PreparedServer, ProxyError> {
    let (config, certificate) = server(config)?;
    Ok(PreparedServer {
        config,
        certificate,
    })
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn server(config: &ServerTls) -> Result<(Arc<ServerConfig>, CertificateInfo), ProxyError> {
    let bytes = read_pem(&config.certificate)?;
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
        fingerprint: format!("{:x}", Sha256::digest(&bytes)),
    };
    let key = read_pem(&config.private_key)?;
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
    client_with_fingerprint(config).map(|(client, _)| client)
}
pub fn client_with_fingerprint(
    config: &UpstreamConfig,
) -> Result<(Arc<ClientConfig>, Option<String>), ProxyError> {
    let mut fingerprint = None;
    let mut roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = &config.ca_certificate {
        let bytes = read_pem(path)?;
        fingerprint = Some(format!("{:x}", Sha256::digest(&bytes)));
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
    Ok((Arc::new(client), fingerprint))
}
