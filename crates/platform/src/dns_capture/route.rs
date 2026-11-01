use std::{
    io,
    net::Ipv6Addr,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::Duration,
};

const PROTOCOL: u8 = 199;
const PRIORITY: u32 = 31053;
pub(super) struct LocalRoute {
    prefix: Ipv6Addr,
}
impl LocalRoute {
    pub fn prepare(prefix: Ipv6Addr) -> io::Result<Self> {
        // A released process may have left its route after its BPF FDs detached.
        // Under the namespace-wide lock only our exact protocol/priority/lo marker
        // can be reclaimed; an unrelated route is never replaced or removed.
        if exists(prefix)? {
            change(prefix, false)?;
        }
        change(prefix, true)?;
        let guard = Self { prefix };
        if !guard.exists()? {
            return Err(io::Error::other(
                "kernel did not retain the owned local route marker",
            ));
        }
        Ok(guard)
    }
    pub fn remove(&self) -> io::Result<()> {
        change(self.prefix, false)
    }
    pub fn exists(&self) -> io::Result<bool> {
        exists(self.prefix)
    }
}
impl Drop for LocalRoute {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}
fn socket() -> io::Result<OwnedFd> {
    // SAFETY: NETLINK_ROUTE socket with automatic port binding; OwnedFd closes it.
    let raw = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let timeout = libc::timeval {
        tv_sec: 3,
        tv_usec: 0,
    };
    if unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&timeout as *const libc::timeval).cast(),
            std::mem::size_of_val(&timeout) as _,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}
fn u16n(bytes: &[u8]) -> u16 {
    u16::from_ne_bytes(bytes[..2].try_into().unwrap())
}
fn u32n(bytes: &[u8]) -> u32 {
    u32::from_ne_bytes(bytes[..4].try_into().unwrap())
}
fn align(n: usize) -> usize {
    (n + 3) & !3
}
fn attribute(message: &mut Vec<u8>, kind: u16, value: &[u8]) {
    message.extend_from_slice(&((value.len() + 4) as u16).to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(value);
    message.resize(align(message.len()), 0);
}
fn message(kind: u16, flags: u16, payload: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(128);
    message.extend_from_slice(&((16 + payload.len()) as u32).to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(&flags.to_ne_bytes());
    message.extend_from_slice(&1u32.to_ne_bytes());
    message.extend_from_slice(&0u32.to_ne_bytes());
    message.extend_from_slice(payload);
    message
}
fn exchange(message: &[u8], dump: bool) -> io::Result<Vec<Vec<u8>>> {
    let fd = socket()?;
    // SAFETY: a zeroed sockaddr_nl with AF_NETLINK addresses the kernel, PID 0.
    let mut kernel: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    kernel.nl_family = libc::AF_NETLINK as _;
    if unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            message.as_ptr().cast(),
            message.len(),
            0,
            (&kernel as *const libc::sockaddr_nl).cast(),
            std::mem::size_of_val(&kernel) as _,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut result = vec![];
    let mut buffer = vec![0; 65536];
    let started = std::time::Instant::now();
    let mut total = 0;
    while started.elapsed() < Duration::from_secs(3) {
        // SAFETY: writable receive buffer, descriptor owned for this operation.
        let len = unsafe {
            libc::recv(
                fd.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                libc::MSG_TRUNC,
            )
        };
        if len < 0 {
            return Err(io::Error::last_os_error());
        }
        if len as usize > buffer.len() {
            return Err(io::Error::other("netlink route dump was truncated"));
        }
        total += len as usize;
        if total > 4 * 1024 * 1024 {
            return Err(io::Error::other("netlink route dump exceeds limit"));
        }
        let mut pos = 0;
        while pos + 16 <= len as usize {
            let record = &buffer[pos..len as usize];
            let size = u32n(record) as usize;
            if size < 16 || size > record.len() {
                return Err(io::Error::other("invalid netlink message length"));
            }
            if u32n(&record[8..]) != 1 {
                pos += align(size);
                continue;
            }
            let kind = u16n(&record[4..]);
            if kind == libc::NLMSG_ERROR as u16 {
                if size < 20 {
                    return Err(io::Error::other("invalid netlink error"));
                }
                let errno = i32::from_ne_bytes(record[16..20].try_into().unwrap());
                return if errno == 0 {
                    Ok(result)
                } else {
                    Err(io::Error::from_raw_os_error(-errno))
                };
            }
            if kind == libc::NLMSG_DONE as u16 {
                return Ok(result);
            }
            result.push(record[16..size].to_vec());
            if !dump {
                return Ok(result);
            }
            pos += align(size);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "route acknowledgment timed out",
    ))
}
fn route(prefix: Ipv6Addr) -> io::Result<Vec<u8>> {
    let mut route = vec![
        libc::AF_INET6 as u8,
        112,
        0,
        0,
        255,
        PROTOCOL,
        0,
        2,
        0,
        0,
        0,
        0,
    ]; // local table, canonical IPv6 universe scope, RTN_LOCAL
    let interface = unsafe { libc::if_nametoindex(c"lo".as_ptr()) };
    if interface == 0 {
        return Err(io::Error::last_os_error());
    }
    attribute(&mut route, 1, &prefix.octets());
    attribute(&mut route, 4, &interface.to_ne_bytes());
    attribute(&mut route, 6, &PRIORITY.to_ne_bytes());
    Ok(route)
}
fn change(prefix: Ipv6Addr, add: bool) -> io::Result<()> {
    let kind = if add {
        libc::RTM_NEWROUTE
    } else {
        libc::RTM_DELROUTE
    };
    let flags = libc::NLM_F_REQUEST
        | libc::NLM_F_ACK
        | if add {
            libc::NLM_F_CREATE | libc::NLM_F_EXCL
        } else {
            0
        };
    exchange(&message(kind, flags as u16, &route(prefix)?), false).map(|_| ())
}
fn exists(prefix: Ipv6Addr) -> io::Result<bool> {
    let payload = [libc::AF_INET6 as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let records = exchange(
        &message(
            libc::RTM_GETROUTE,
            (libc::NLM_F_REQUEST | libc::NLM_F_DUMP) as u16,
            &payload,
        ),
        true,
    )?;
    let expected = route(prefix)?;
    Ok(records.iter().any(|record| {
        if record.len() < 12 || record[..8] != expected[..8] {
            return false;
        }
        let attrs = |bytes: &[u8]| {
            let mut attrs = std::collections::BTreeMap::new();
            let mut pos = 12;
            while pos + 4 <= bytes.len() {
                let len = u16n(&bytes[pos..]) as usize;
                if len < 4 || pos + len > bytes.len() {
                    break;
                }
                attrs.insert(u16n(&bytes[pos + 2..]), bytes[pos + 4..pos + len].to_vec());
                pos += align(len);
            }
            attrs
        };
        let attrs = attrs(record);
        let needed = attrs_for_expected(&expected);
        needed
            .iter()
            .all(|(key, value)| attrs.get(key) == Some(value))
    }))
}
fn attrs_for_expected(bytes: &[u8]) -> std::collections::BTreeMap<u16, Vec<u8>> {
    let mut result = std::collections::BTreeMap::new();
    let mut pos = 12;
    while pos + 4 <= bytes.len() {
        let len = u16n(&bytes[pos..]) as usize;
        result.insert(u16n(&bytes[pos + 2..]), bytes[pos + 4..pos + len].to_vec());
        pos += align(len);
    }
    result
}
