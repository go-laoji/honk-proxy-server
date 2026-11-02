use crate::{BoxIo, ProxyError};
use hps_config::UpstreamCredentials;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Supplier authentication; this is not a project account or ingress login.
pub(crate) async fn authenticate(
    io: &mut BoxIo,
    credentials: Option<&UpstreamCredentials>,
) -> Result<(), ProxyError> {
    let methods: &[u8] = if credentials.is_some() {
        &[5, 2, 0, 2]
    } else {
        &[5, 1, 0]
    };
    io.write_all(methods).await?;
    io.flush().await?;
    let mut choice = [0; 2];
    io.read_exact(&mut choice).await?;
    if choice == [5, 2] {
        let credentials = credentials.ok_or(ProxyError::Authentication)?;
        if credentials.username.is_empty()
            || credentials.password.is_empty()
            || credentials.username.len() > 255
            || credentials.password.len() > 255
        {
            return Err(ProxyError::Authentication);
        }
        let mut auth = vec![1, credentials.username.len() as u8];
        auth.extend(credentials.username.as_bytes());
        auth.push(credentials.password.len() as u8);
        auth.extend(credentials.password.as_bytes());
        io.write_all(&auth).await?;
        io.flush().await?;
        io.read_exact(&mut choice).await?;
        if choice != [1, 0] {
            return Err(ProxyError::Authentication);
        }
    } else if choice != [5, 0] {
        return Err(ProxyError::Authentication);
    }
    Ok(())
}
