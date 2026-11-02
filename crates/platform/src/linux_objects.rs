//! Kernel namespace identity and OS resource ownership, separate from the open
//! project management API. Only compile-time internal scopes choose lock names.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    net::UdpSocket,
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::Path,
};
pub(crate) fn netns_cookie() -> io::Result<u64> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    let mut value = 0u64;
    let mut size = std::mem::size_of_val(&value) as libc::socklen_t;
    // SAFETY: live descriptor and correctly sized initialized integer storage.
    if unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NETNS_COOKIE,
            (&mut value as *mut u64).cast(),
            &mut size,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if value == 0 || size as usize != std::mem::size_of::<u64>() {
        return Err(io::Error::other("invalid network namespace cookie"));
    }
    Ok(value)
}
pub(crate) fn ownership(scope: &str, cookie: u64) -> io::Result<File> {
    if !matches!(scope, "dns" | "transparent") || cookie == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid kernel object scope",
        ));
    }
    let path = Path::new("/run/hps");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "capture requires a private /run/hps directory owned by the daemon OS user",
        ));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path.join(format!("{scope}-netns-{cookie}.lock")))?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "capture ownership file must be private and owned by the daemon OS user",
        ));
    }
    fs2::FileExt::try_lock_exclusive(&file)?;
    file.set_len(0)?;
    writeln!(file, "{}", std::process::id())?;
    Ok(file)
}
