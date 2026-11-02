use crate::{BoxIo, ProxyError, runtime::Session, target::Target, traffic::relay};
use std::net::{Ipv4Addr, Ipv6Addr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) async fn serve(mut io: BoxIo, session: Session) -> Result<(), ProxyError> {
    let request = match tokio::time::timeout(session.handshake_timeout(), handshake(&mut io)).await
    {
        Ok(result) => result?,
        Err(_) => return Err(ProxyError::Timeout),
    };
    let target = match request {
        Request::Connect(target) => target,
        Request::Udp { address, port } => {
            return super::socks_udp::serve(io, session, address, port).await;
        }
    };
    let upstream = match session
        .dialer
        .connect_context(&target, false, &session.selection_context)
        .await
    {
        Ok(connected) => connected,
        Err(error) => {
            reply(
                &mut io,
                match error {
                    ProxyError::Rejected => 2,
                    ProxyError::NoUpstream => 3,
                    ProxyError::Timeout => 6,
                    _ => 5,
                },
            )
            .await?;
            return Err(error);
        }
    };
    if let Some(observation) = upstream.observation.as_ref() {
        observation.bind_inbound_cancel(session.cancel.clone());
    }
    reply(&mut io, 0).await?;
    relay(io, upstream.io, session.limits.buffer_bytes, session.budget).await?;
    Ok(())
}
async fn reply(io: &mut BoxIo, code: u8) -> Result<(), ProxyError> {
    io.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
    io.flush().await?;
    Ok(())
}
enum Request {
    Connect(Target),
    Udp {
        address: std::net::IpAddr,
        port: u16,
    },
}
async fn handshake(io: &mut BoxIo) -> Result<Request, ProxyError> {
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
    if ![1, 3].contains(&prefix[1]) {
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
    if prefix[1] == 3 {
        return match host.parse() {
            Ok(address) => Ok(Request::Udp { address, port }),
            Err(_) => {
                reply(io, 8).await?;
                Err(ProxyError::InvalidTarget)
            }
        };
    }
    match Target::new(&host, port) {
        Ok(target) => Ok(Request::Connect(target)),
        Err(error) => {
            reply(io, 8).await?;
            Err(error)
        }
    }
}
