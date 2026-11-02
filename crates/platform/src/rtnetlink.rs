//! Small bounded route-netlink transactions, with exact sequence and kernel
//! sender checks. Each request owns a fresh descriptor and has a finite deadline.
use std::{
    collections::BTreeMap,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};
static SEQUENCE: AtomicU32 = AtomicU32::new(1);
pub(crate) struct Message {
    pub kind: u16,
    pub payload: Vec<u8>,
}
pub(crate) fn u16n(bytes: &[u8]) -> u16 {
    u16::from_ne_bytes(bytes[..2].try_into().unwrap())
}
pub(crate) fn u32n(bytes: &[u8]) -> u32 {
    u32::from_ne_bytes(bytes[..4].try_into().unwrap())
}
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "malformed or interrupted route-netlink response",
    )
}
fn align(length: usize) -> usize {
    (length + 3) & !3
}
pub(crate) fn attribute(output: &mut Vec<u8>, kind: u16, value: &[u8]) {
    output.extend_from_slice(&((value.len() + 4) as u16).to_ne_bytes());
    output.extend_from_slice(&kind.to_ne_bytes());
    output.extend_from_slice(value);
    output.resize(align(output.len()), 0);
}
pub(crate) fn attributes(payload: &[u8], offset: usize) -> io::Result<BTreeMap<u16, Vec<u8>>> {
    if offset > payload.len() {
        return Err(invalid());
    }
    let mut result = BTreeMap::new();
    let mut position = offset;
    while position < payload.len() {
        if payload.len() - position < 4 {
            return Err(invalid());
        }
        let length = usize::from(u16n(&payload[position..]));
        if length < 4 || length > payload.len() - position {
            return Err(invalid());
        }
        let kind = u16n(&payload[position + 2..]) & 0x3fff;
        if kind != 0 {
            let value = payload[position + 4..position + length].to_vec();
            // IPv6 route-get can emit RTA_PREFSRC twice (selected source and
            // route preference). Identical bytes are unambiguous; conflicting
            // duplicates must never change an ownership decision by ordering.
            if let Some(previous) = result.insert(kind, value.clone())
                && previous != value
            {
                return Err(invalid());
            }
        }
        let next = position + align(length);
        if next > payload.len() && position + length != payload.len() {
            return Err(invalid());
        }
        position = next;
    }
    Ok(result)
}
fn socket() -> io::Result<OwnedFd> {
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let timeout = libc::timeval {
        tv_sec: 2,
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
pub(crate) fn exchange(
    kind: u16,
    flags: u16,
    payload: &[u8],
    dump: bool,
) -> io::Result<Vec<Message>> {
    let fd = socket()?;
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut request = Vec::with_capacity(16 + payload.len());
    request.extend_from_slice(&((16 + payload.len()) as u32).to_ne_bytes());
    request.extend_from_slice(&kind.to_ne_bytes());
    request.extend_from_slice(&flags.to_ne_bytes());
    request.extend_from_slice(&sequence.to_ne_bytes());
    request.extend_from_slice(&0u32.to_ne_bytes());
    request.extend_from_slice(payload);
    // SAFETY: initialized kernel sockaddr and a live request/descriptor.
    let mut kernel: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    kernel.nl_family = libc::AF_NETLINK as _;
    let sent = unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            request.as_ptr().cast(),
            request.len(),
            0,
            (&kernel as *const libc::sockaddr_nl).cast(),
            std::mem::size_of_val(&kernel) as _,
        )
    };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    if sent as usize != request.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "incomplete route-netlink request",
        ));
    }
    let mut result = vec![];
    let mut buffer = vec![0; 65536];
    let mut total = 0usize;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        let mut sender: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of_val(&sender) as libc::socklen_t;
        let received = unsafe {
            libc::recvfrom(
                fd.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                libc::MSG_TRUNC,
                (&mut sender as *mut libc::sockaddr_nl).cast(),
                &mut size,
            )
        };
        if received < 0 {
            return Err(io::Error::last_os_error());
        }
        if size as usize != std::mem::size_of_val(&sender)
            || sender.nl_family != libc::AF_NETLINK as libc::sa_family_t
            || sender.nl_pid != 0
            || received as usize > buffer.len()
        {
            return Err(invalid());
        }
        total = total.checked_add(received as usize).ok_or_else(invalid)?;
        if total > 4 * 1024 * 1024 {
            return Err(io::Error::other(
                "route-netlink dump exceeds bounded capacity",
            ));
        }
        let mut position = 0usize;
        while position < received as usize {
            if received as usize - position < 16 {
                return Err(invalid());
            }
            let record = &buffer[position..received as usize];
            let length = u32n(record) as usize;
            if length < 16 || length > record.len() {
                return Err(invalid());
            }
            if u32n(&record[8..]) != sequence {
                position += align(length);
                continue;
            }
            let kind = u16n(&record[4..]);
            let flags = u16n(&record[6..]);
            if flags & libc::NLM_F_DUMP_INTR as u16 != 0 {
                return Err(invalid());
            }
            if kind == libc::NLMSG_OVERRUN as u16 {
                return Err(invalid());
            }
            if kind == libc::NLMSG_ERROR as u16 {
                if length < 20 {
                    return Err(invalid());
                }
                let code = i32::from_ne_bytes(record[16..20].try_into().unwrap());
                if code != 0 {
                    return Err(io::Error::from_raw_os_error(-code));
                }
                if !dump {
                    return Ok(result);
                }
            } else if kind == libc::NLMSG_DONE as u16 {
                if length >= 20 {
                    let code = i32::from_ne_bytes(record[16..20].try_into().unwrap());
                    if code != 0 {
                        return Err(io::Error::from_raw_os_error(-code));
                    }
                }
                return Ok(result);
            } else if kind != libc::NLMSG_NOOP as u16 {
                result.push(Message {
                    kind,
                    payload: record[16..length].to_vec(),
                });
            }
            position += align(length);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "route-netlink response deadline exceeded",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attributes_allow_identical_kernel_duplicates_but_reject_conflicting_or_truncated_fields() {
        let mut bytes = vec![0; 12];
        attribute(&mut bytes, 6, &31530u32.to_ne_bytes());
        attribute(&mut bytes, 21, &[200]);
        let parsed = attributes(&bytes, 12).unwrap();
        assert_eq!(parsed[&6], 31530u32.to_ne_bytes());
        assert_eq!(parsed[&21], [200]);
        for length in 13..20 {
            assert!(attributes(&bytes[..length], 12).is_err());
        }
        attribute(&mut bytes, 6, &31530u32.to_ne_bytes());
        assert!(attributes(&bytes, 12).is_ok());
        attribute(&mut bytes, 6, &1u32.to_ne_bytes());
        assert!(attributes(&bytes, 12).is_err());
    }
}
