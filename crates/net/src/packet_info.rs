//! Linux UDP destination/source preservation for wildcard DNS listeners.
use std::{
    io, mem,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6},
    os::fd::AsRawFd,
};
use tokio::{
    io::Interest,
    net::{TcpListener, UdpSocket},
};

#[derive(Debug, Clone, Copy)]
pub struct PacketInfo {
    pub destination: IpAddr,
    pub interface: u32,
}
fn option(fd: i32, level: i32, name: i32, value: i32) -> io::Result<()> {
    // SAFETY: live socket descriptor and a correctly sized integer option.
    if unsafe {
        libc::setsockopt(
            fd,
            level,
            name,
            (&value as *const i32).cast(),
            mem::size_of::<i32>() as _,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub fn enable(socket: &UdpSocket) -> io::Result<()> {
    if socket.local_addr()?.is_ipv4() {
        option(socket.as_raw_fd(), libc::IPPROTO_IP, libc::IP_PKTINFO, 1)
    } else {
        option(
            socket.as_raw_fd(),
            libc::IPPROTO_IPV6,
            libc::IPV6_RECVPKTINFO,
            1,
        )
    }
}
fn capture_socket(address: SocketAddr, kind: socket2::Type) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(socket2::Domain::for_address(address), kind, None)?;
    if kind == socket2::Type::STREAM {
        socket.set_reuse_address(true)?;
    }
    socket.set_nonblocking(true)?;
    if address.is_ipv6() {
        socket.set_only_v6(true)?;
        option(
            socket.as_raw_fd(),
            libc::IPPROTO_IPV6,
            libc::IPV6_FREEBIND,
            1,
        )?;
        option(
            socket.as_raw_fd(),
            libc::IPPROTO_IPV6,
            libc::IPV6_TRANSPARENT,
            1,
        )?;
    }
    // Only local loopback arrivals can reach the capture listener, even though
    // different resolver aliases require a wildcard bind.
    socket.bind_device(Some(b"lo"))?;
    socket.bind(&address.into())?;
    Ok(socket)
}
pub fn bind_capture_tcp(address: SocketAddr) -> io::Result<TcpListener> {
    let socket = capture_socket(address, socket2::Type::STREAM)?;
    socket.listen(128)?;
    TcpListener::from_std(socket.into())
}
pub fn bind_capture_udp(address: SocketAddr) -> io::Result<UdpSocket> {
    let socket = capture_socket(address, socket2::Type::DGRAM)?;
    let socket = UdpSocket::from_std(socket.into())?;
    enable(&socket)?;
    Ok(socket)
}
pub async fn recv(
    socket: &UdpSocket,
    buffer: &mut [u8],
) -> io::Result<(usize, SocketAddr, PacketInfo)> {
    socket
        .async_io(Interest::READABLE, || {
            // SAFETY: all syscall pointers refer to live, suitably aligned storage for
            // this call. Ancillary lengths are checked before reading a typed payload.
            unsafe {
                let mut peer: libc::sockaddr_storage = mem::zeroed();
                let mut control = [0usize; 32];
                let mut iov = libc::iovec {
                    iov_base: buffer.as_mut_ptr().cast(),
                    iov_len: buffer.len(),
                };
                let mut header: libc::msghdr = mem::zeroed();
                header.msg_name = (&mut peer as *mut libc::sockaddr_storage).cast();
                header.msg_namelen = mem::size_of_val(&peer) as _;
                header.msg_iov = &mut iov;
                header.msg_iovlen = 1;
                header.msg_control = control.as_mut_ptr().cast();
                header.msg_controllen = mem::size_of_val(&control) as _;
                let len = libc::recvmsg(socket.as_raw_fd(), &mut header, libc::MSG_DONTWAIT);
                if len < 0 {
                    return Err(io::Error::last_os_error());
                }
                if header.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "truncated DNS datagram metadata",
                    ));
                }
                let peer = match peer.ss_family as i32 {
                    libc::AF_INET
                        if header.msg_namelen as usize >= mem::size_of::<libc::sockaddr_in>() =>
                    {
                        let value = *(std::ptr::addr_of!(peer).cast::<libc::sockaddr_in>());
                        SocketAddr::new(
                            IpAddr::V4(Ipv4Addr::from(value.sin_addr.s_addr.to_ne_bytes())),
                            u16::from_be(value.sin_port),
                        )
                    }
                    libc::AF_INET6
                        if header.msg_namelen as usize >= mem::size_of::<libc::sockaddr_in6>() =>
                    {
                        let value = *(std::ptr::addr_of!(peer).cast::<libc::sockaddr_in6>());
                        SocketAddr::V6(SocketAddrV6::new(
                            Ipv6Addr::from(value.sin6_addr.s6_addr),
                            u16::from_be(value.sin6_port),
                            value.sin6_flowinfo,
                            value.sin6_scope_id,
                        ))
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unknown DNS peer family",
                        ));
                    }
                };
                let mut entry = libc::CMSG_FIRSTHDR(&header);
                while !entry.is_null() {
                    let record = &*entry;
                    if record.cmsg_level == libc::IPPROTO_IP
                        && record.cmsg_type == libc::IP_PKTINFO
                        && record.cmsg_len as usize
                            >= libc::CMSG_LEN(mem::size_of::<libc::in_pktinfo>() as _) as usize
                    {
                        let value = std::ptr::read_unaligned(
                            libc::CMSG_DATA(entry).cast::<libc::in_pktinfo>(),
                        );
                        return Ok((
                            len as usize,
                            peer,
                            PacketInfo {
                                destination: IpAddr::V4(Ipv4Addr::from(
                                    value.ipi_addr.s_addr.to_ne_bytes(),
                                )),
                                interface: value.ipi_ifindex as u32,
                            },
                        ));
                    }
                    if record.cmsg_level == libc::IPPROTO_IPV6
                        && record.cmsg_type == libc::IPV6_PKTINFO
                        && record.cmsg_len as usize
                            >= libc::CMSG_LEN(mem::size_of::<libc::in6_pktinfo>() as _) as usize
                    {
                        let value = std::ptr::read_unaligned(
                            libc::CMSG_DATA(entry).cast::<libc::in6_pktinfo>(),
                        );
                        return Ok((
                            len as usize,
                            peer,
                            PacketInfo {
                                destination: IpAddr::V6(Ipv6Addr::from(value.ipi6_addr.s6_addr)),
                                interface: value.ipi6_ifindex,
                            },
                        ));
                    }
                    entry = libc::CMSG_NXTHDR(&header, entry);
                }
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "DNS destination metadata missing",
                ))
            }
        })
        .await
}
pub async fn send(
    socket: &UdpSocket,
    buffer: &[u8],
    peer: SocketAddr,
    info: PacketInfo,
) -> io::Result<usize> {
    socket
        .async_io(Interest::WRITABLE, || {
            // SAFETY: initialized socket address, iovec and aligned ancillary storage.
            // The socket option enabled in bind_capture_udp permits the owned IPv6 aliases.
            unsafe {
                let address = socket2::SockAddr::from(peer);
                let mut control = [0usize; 32];
                let mut iov = libc::iovec {
                    iov_base: buffer.as_ptr().cast_mut().cast(),
                    iov_len: buffer.len(),
                };
                let mut header: libc::msghdr = mem::zeroed();
                header.msg_name = address.as_ptr().cast_mut().cast();
                header.msg_namelen = address.len();
                header.msg_iov = &mut iov;
                header.msg_iovlen = 1;
                header.msg_control = control.as_mut_ptr().cast();
                header.msg_controllen = mem::size_of_val(&control) as _;
                let entry = libc::CMSG_FIRSTHDR(&header);
                match info.destination {
                    IpAddr::V4(ip) => {
                        (*entry).cmsg_level = libc::IPPROTO_IP;
                        (*entry).cmsg_type = libc::IP_PKTINFO;
                        (*entry).cmsg_len =
                            libc::CMSG_LEN(mem::size_of::<libc::in_pktinfo>() as _) as _;
                        let value = libc::in_pktinfo {
                            ipi_ifindex: info.interface as i32,
                            ipi_spec_dst: libc::in_addr {
                                s_addr: u32::from_ne_bytes(ip.octets()),
                            },
                            ipi_addr: libc::in_addr { s_addr: 0 },
                        };
                        std::ptr::write_unaligned(
                            libc::CMSG_DATA(entry).cast::<libc::in_pktinfo>(),
                            value,
                        );
                        header.msg_controllen =
                            libc::CMSG_SPACE(mem::size_of::<libc::in_pktinfo>() as _) as _;
                    }
                    IpAddr::V6(ip) => {
                        (*entry).cmsg_level = libc::IPPROTO_IPV6;
                        (*entry).cmsg_type = libc::IPV6_PKTINFO;
                        (*entry).cmsg_len =
                            libc::CMSG_LEN(mem::size_of::<libc::in6_pktinfo>() as _) as _;
                        let value = libc::in6_pktinfo {
                            ipi6_addr: libc::in6_addr {
                                s6_addr: ip.octets(),
                            },
                            ipi6_ifindex: info.interface,
                        };
                        std::ptr::write_unaligned(
                            libc::CMSG_DATA(entry).cast::<libc::in6_pktinfo>(),
                            value,
                        );
                        header.msg_controllen =
                            libc::CMSG_SPACE(mem::size_of::<libc::in6_pktinfo>() as _) as _;
                    }
                }
                let count = libc::sendmsg(
                    socket.as_raw_fd(),
                    &header,
                    libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
                );
                if count < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(count as usize)
                }
            }
        })
        .await
}
