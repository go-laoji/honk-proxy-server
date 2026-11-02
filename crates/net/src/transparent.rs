//! Linux transparent sockets. Binding/listening is separate from enabling any
//! eBPF hook or policy route; only the capture owner publishes these descriptors.
use std::{
    io, mem,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    os::fd::{AsRawFd, RawFd},
};
use tokio::{
    io::Interest,
    net::{TcpListener, UdpSocket},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalDatagram {
    pub bytes: usize,
    pub source: SocketAddr,
    pub destination: SocketAddr,
}
fn option(fd: RawFd, level: i32, name: i32, value: i32) -> io::Result<()> {
    // SAFETY: the descriptor is borrowed and the option uses an initialized i32.
    let result = unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            (&value as *const i32).cast(),
            mem::size_of::<i32>() as _,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
fn socket(
    address: SocketAddr,
    kind: socket2::Type,
    bypass_mark: u32,
) -> io::Result<socket2::Socket> {
    if bypass_mark == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transparent socket requires an explicit bypass mark",
        ));
    }
    let socket = socket2::Socket::new(socket2::Domain::for_address(address), kind, None)?;
    socket.set_nonblocking(true)?;
    socket.set_reuse_address(true)?;
    socket.set_mark(bypass_mark)?;
    if address.is_ipv4() {
        option(
            socket.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_TRANSPARENT,
            1,
        )?;
    } else {
        socket.set_only_v6(true)?;
        option(
            socket.as_raw_fd(),
            libc::IPPROTO_IPV6,
            libc::IPV6_TRANSPARENT,
            1,
        )?;
    }
    socket.bind(&address.into())?;
    Ok(socket)
}
pub fn bind_tcp(address: SocketAddr, bypass_mark: u32, backlog: u32) -> io::Result<TcpListener> {
    if !(1..=4096).contains(&backlog) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transparent TCP backlog must be between 1 and 4096",
        ));
    }
    let socket = socket(address, socket2::Type::STREAM, bypass_mark)?;
    socket.listen(backlog as i32)?;
    TcpListener::from_std(socket.into())
}
pub fn bind_udp(address: SocketAddr, bypass_mark: u32) -> io::Result<UdpSocket> {
    let socket = socket(address, socket2::Type::DGRAM, bypass_mark)?;
    enable_original_destination(socket.as_raw_fd(), address.is_ipv4())?;
    UdpSocket::from_std(socket.into())
}
/// A separate connected reply socket binds the full original source address and
/// port. The TC UDP path must keep assigning captured requests to the main socket.
pub fn bind_udp_reply(
    original: SocketAddr,
    client: SocketAddr,
    bypass_mark: u32,
) -> io::Result<UdpSocket> {
    if original.is_ipv4() != client.is_ipv4() || original.port() == 0 || client.port() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid transparent UDP reply endpoints",
        ));
    }
    let socket = socket(original, socket2::Type::DGRAM, bypass_mark)?;
    socket.connect(&client.into())?;
    UdpSocket::from_std(socket.into())
}
fn enable_original_destination(fd: RawFd, ipv4: bool) -> io::Result<()> {
    if ipv4 {
        option(fd, libc::IPPROTO_IP, libc::IP_RECVORIGDSTADDR, 1)
    } else {
        option(fd, libc::IPPROTO_IPV6, libc::IPV6_RECVORIGDSTADDR, 1)
    }
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "missing, truncated or inconsistent original UDP destination",
    )
}
unsafe fn address(raw: *const u8, size: usize) -> io::Result<SocketAddr> {
    if size < mem::size_of::<libc::sa_family_t>() {
        return Err(invalid());
    }
    // SAFETY: callers supply a live initialized region of the checked size; its
    // alignment is not assumed for either the family or the sockaddr payload.
    let family = unsafe { std::ptr::read_unaligned(raw.cast::<libc::sa_family_t>()) };
    match i32::from(family) {
        libc::AF_INET if size >= mem::size_of::<libc::sockaddr_in>() => {
            let value = unsafe { std::ptr::read_unaligned(raw.cast::<libc::sockaddr_in>()) };
            Ok(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::from(value.sin_addr.s_addr.to_ne_bytes())),
                u16::from_be(value.sin_port),
            ))
        }
        libc::AF_INET6 if size >= mem::size_of::<libc::sockaddr_in6>() => {
            let value = unsafe { std::ptr::read_unaligned(raw.cast::<libc::sockaddr_in6>()) };
            Ok(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(value.sin6_addr.s6_addr),
                u16::from_be(value.sin6_port),
                value.sin6_flowinfo,
                value.sin6_scope_id,
            )))
        }
        _ => Err(invalid()),
    }
}
unsafe fn destination(header: &libc::msghdr) -> io::Result<SocketAddr> {
    // SAFETY: the kernel populated this bounded, initialized ancillary buffer.
    let mut entry = unsafe { libc::CMSG_FIRSTHDR(header) };
    let start = header.msg_control as usize;
    let end = start
        .checked_add(header.msg_controllen as usize)
        .ok_or_else(invalid)?;
    let mut found = None;
    while !entry.is_null() {
        let position = entry as usize;
        if position < start || position > end || mem::size_of::<libc::cmsghdr>() > end - position {
            return Err(invalid());
        }
        let record = unsafe { &*entry };
        let length = record.cmsg_len as usize;
        let prefix = unsafe { libc::CMSG_LEN(0) } as usize;
        if length < prefix || length > end - position {
            return Err(invalid());
        }
        let ipv4 =
            record.cmsg_level == libc::IPPROTO_IP && record.cmsg_type == libc::IP_ORIGDSTADDR;
        let ipv6 =
            record.cmsg_level == libc::IPPROTO_IPV6 && record.cmsg_type == libc::IPV6_ORIGDSTADDR;
        if ipv4 || ipv6 {
            if found.is_some() {
                return Err(invalid());
            }
            let value = unsafe { address(libc::CMSG_DATA(entry), length - prefix) }?;
            if value.is_ipv4() != ipv4 || value.port() == 0 || value.ip().is_unspecified() {
                return Err(invalid());
            }
            found = Some(value);
        }
        entry = unsafe { libc::CMSG_NXTHDR(header, entry) };
    }
    found.ok_or_else(invalid)
}
/// Receives only complete datagrams with a full original address and port. The
/// caller owns the bounded buffer; an oversized message is consumed and rejected.
pub async fn recv_original(socket: &UdpSocket, buffer: &mut [u8]) -> io::Result<OriginalDatagram> {
    socket
        .async_io(Interest::READABLE, || {
            // SAFETY: the syscall's iovec, source and aligned control storage are live
            // and initialized. No pointer or reference escapes this invocation.
            unsafe {
                let mut source: libc::sockaddr_storage = mem::zeroed();
                let mut control = [0usize; 32];
                let mut iov = libc::iovec {
                    iov_base: buffer.as_mut_ptr().cast(),
                    iov_len: buffer.len(),
                };
                let mut header: libc::msghdr = mem::zeroed();
                header.msg_name = (&mut source as *mut libc::sockaddr_storage).cast();
                header.msg_namelen = mem::size_of_val(&source) as _;
                header.msg_iov = &mut iov;
                header.msg_iovlen = 1;
                header.msg_control = control.as_mut_ptr().cast();
                header.msg_controllen = mem::size_of_val(&control) as _;
                let length = libc::recvmsg(socket.as_raw_fd(), &mut header, libc::MSG_DONTWAIT);
                if length < 0 {
                    return Err(io::Error::last_os_error());
                }
                if header.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0
                    || length as usize > buffer.len()
                    || header.msg_controllen as usize > mem::size_of_val(&control)
                    || header.msg_namelen as usize > mem::size_of_val(&source)
                {
                    return Err(invalid());
                }
                let source = address(
                    (&source as *const libc::sockaddr_storage).cast(),
                    header.msg_namelen as usize,
                )?;
                let destination = destination(&header)?;
                if source.is_ipv4() != destination.is_ipv4() || source.port() == 0 {
                    return Err(invalid());
                }
                Ok(OriginalDatagram {
                    bytes: length as usize,
                    source,
                    destination,
                })
            }
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[tokio::test]
    async fn udp_recovers_actual_dual_stack_addresses_and_ports_and_rejects_partial_payload() {
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            let receiver = UdpSocket::bind(SocketAddr::new(ip, 0)).await.unwrap();
            enable_original_destination(receiver.as_raw_fd(), ip.is_ipv4()).unwrap();
            let sender = UdpSocket::bind(SocketAddr::new(ip, 0)).await.unwrap();
            sender
                .send_to(b"first-packet", receiver.local_addr().unwrap())
                .await
                .unwrap();
            let mut buffer = [0; 64];
            let received = tokio::time::timeout(
                Duration::from_secs(2),
                recv_original(&receiver, &mut buffer),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(&buffer[..received.bytes], b"first-packet");
            assert_eq!(received.source, sender.local_addr().unwrap());
            assert_eq!(received.destination, receiver.local_addr().unwrap());
            sender
                .send_to(&[7; 65], receiver.local_addr().unwrap())
                .await
                .unwrap();
            assert_eq!(
                recv_original(&receiver, &mut buffer)
                    .await
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidData
            );
            sender
                .send_to(b"next", receiver.local_addr().unwrap())
                .await
                .unwrap();
            assert_eq!(
                recv_original(&receiver, &mut buffer).await.unwrap().bytes,
                4
            );
            assert_eq!(&buffer[..4], b"next");
        }
    }
    #[tokio::test]
    async fn udp_without_original_destination_never_invents_a_target_from_listener() {
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        sender
            .send_to(b"test", receiver.local_addr().unwrap())
            .await
            .unwrap();
        let mut buffer = [0; 64];
        assert_eq!(
            recv_original(&receiver, &mut buffer)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
    #[tokio::test]
    #[ignore = "requires Linux CAP_NET_RAW or CAP_NET_ADMIN for transparent sockets and SO_MARK"]
    async fn privileged_transparent_sockets_bind_nonlocal_endpoints_with_exact_bypass_mark() {
        let mark = 0x4850_0001;
        for address in ["198.18.1.2:0", "[2001:db8:4850::2]:0"] {
            let address: SocketAddr = address.parse().unwrap();
            let tcp = bind_tcp(address, mark, 128).unwrap();
            assert_eq!(tcp.local_addr().unwrap().ip(), address.ip());
            let udp = bind_udp(address, mark).unwrap();
            assert_eq!(udp.local_addr().unwrap().ip(), address.ip());
            for fd in [tcp.as_raw_fd(), udp.as_raw_fd()] {
                let mut actual = 0u32;
                let mut size = mem::size_of_val(&actual) as libc::socklen_t;
                assert_eq!(
                    unsafe {
                        libc::getsockopt(
                            fd,
                            libc::SOL_SOCKET,
                            libc::SO_MARK,
                            (&mut actual as *mut u32).cast(),
                            &mut size,
                        )
                    },
                    0
                );
                assert_eq!(actual, mark);
            }
        }
        let reservation = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let original = reservation.local_addr().unwrap();
        drop(reservation);
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let reply = bind_udp_reply(original, client.local_addr().unwrap(), mark).unwrap();
        reply.send(b"original-source-reply").await.unwrap();
        let mut buffer = [0; 64];
        let (length, source) =
            tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(source, original);
        assert_eq!(&buffer[..length], b"original-source-reply");
        assert!(bind_tcp("127.0.0.1:0".parse().unwrap(), 0, 128).is_err());
    }
}
