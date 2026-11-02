use crate::{BoxIo, ProxyError, runtime::Session, target::Target, traffic::relay};
use std::net::{Ipv4Addr, Ipv6Addr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) async fn serve(mut io: BoxIo, session: Session) -> Result<(), ProxyError> {
    let target = match tokio::time::timeout(session.handshake_timeout(), handshake(&mut io)).await {
        Ok(result) => result?,
        Err(_) => return Err(ProxyError::Timeout),
    };
    let upstream = match session.dialer.connect(&target, false).await {
        Ok(connected) => connected,
        Err(error) => {
            reply(
                &mut io,
                match error {
                    ProxyError::NoUpstream => 3,
                    ProxyError::Timeout => 6,
                    _ => 5,
                },
            )
            .await?;
            return Err(error);
        }
    };
    reply(&mut io, 0).await?;
    relay(io, upstream.io, session.limits.buffer_bytes, session.budget).await?;
    Ok(())
}
async fn reply(io: &mut BoxIo, code: u8) -> Result<(), ProxyError> {
    io.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    io.flush().await?;
    Ok(())
}
async fn handshake(io: &mut BoxIo) -> Result<Target, ProxyError> {
    if io.read_u8().await? != 5 {
        return Err(ProxyError::InvalidTarget);
    }
    let count = io.read_u8().await?;
    let mut methods = vec![0; count as usize];
    io.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        io.write_all(&[5, 255]).await?;
        io.flush().await?;
        return Err(ProxyError::InvalidTarget);
    }
    io.write_all(&[5, 0]).await?;
    io.flush().await?;
    let mut prefix = [0; 4];
    io.read_exact(&mut prefix).await?;
    if prefix[0] != 5 || prefix[2] != 0 {
        reply(io, 1).await?;
        return Err(ProxyError::InvalidTarget);
    }
    if prefix[1] != 1 {
        reply(io, 7).await?;
        return Err(ProxyError::InvalidTarget);
    }
    let host = match prefix[3] {
        1 => {
            let mut bytes = [0; 4];
            io.read_exact(&mut bytes).await?;
            Ipv4Addr::from(bytes).to_string()
        }
        4 => {
            let mut bytes = [0; 16];
            io.read_exact(&mut bytes).await?;
            Ipv6Addr::from(bytes).to_string()
        }
        3 => {
            let len = io.read_u8().await?;
            let mut bytes = vec![0; len as usize];
            io.read_exact(&mut bytes).await?;
            String::from_utf8(bytes).map_err(|_| ProxyError::InvalidTarget)?
        }
        _ => {
            reply(io, 8).await?;
            return Err(ProxyError::InvalidTarget);
        }
    };
    let port = io.read_u16().await?;
    match Target::new(&host, port) {
        Ok(target) => Ok(target),
        Err(error) => {
            reply(io, 8).await?;
            Err(error)
        }
    }
}
