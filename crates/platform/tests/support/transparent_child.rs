use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, UdpSocket},
    path::Path,
    time::Duration,
};

pub fn run() {
    let group = std::env::var("HPS_TRANSPARENT_CHILD_CGROUP").unwrap();
    let group = Path::new(&group).canonicalize().unwrap();
    assert!(
        group
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hps-transparent-")
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(group.join("cgroup.procs"))
        .unwrap()
        .write_all(format!("{}\n", std::process::id()).as_bytes())
        .unwrap();
    // Already belongs to the selected cgroup, but created while capture is
    // disabled. The parent activates only after this readiness barrier.
    let previous = [
        UdpSocket::bind("0.0.0.0:0").unwrap(),
        UdpSocket::bind("[::]:0").unwrap(),
    ];
    println!("HPS_TRANSPARENT_CHILD_READY");
    std::io::stdout().flush().unwrap();
    let mut enabled = [0];
    std::io::stdin().read_exact(&mut enabled).unwrap();
    assert_eq!(enabled, [1]);
    for (tcp, udp, bind) in [
        (
            "203.0.113.77:8443",
            ["203.0.113.78:5353", "198.51.100.79:5354"],
            "0.0.0.0:0",
        ),
        (
            "[2001:db8::77]:8443",
            ["[2001:db8::78]:5353", "[2001:db8::79]:5354"],
            "[::]:0",
        ),
    ] {
        let target: SocketAddr = tcp.parse().unwrap();
        let mut tcp = TcpStream::connect_timeout(&target, Duration::from_secs(3)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        assert_eq!(tcp.peer_addr().unwrap(), target);
        tcp.write_all(b"tcp-original-target").unwrap();
        let mut reply = [0; 19];
        tcp.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"TCP-original-target");
        for connected in [false, true] {
            let socket = UdpSocket::bind(bind).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let targets = udp.map(|p| p.parse::<SocketAddr>().unwrap());
            if connected {
                socket.connect(targets[0]).unwrap();
            }
            for target in targets {
                if connected {
                    socket.connect(target).unwrap();
                    socket.send(b"udp-original-target").unwrap();
                } else {
                    socket.send_to(b"udp-original-target", target).unwrap();
                }
                let mut reply = [0; 64];
                let (length, source) = socket.recv_from(&mut reply).unwrap();
                assert_eq!(source, target);
                assert_eq!(&reply[..length], b"UDP-original-target");
            }
        }
    }
    for (socket, target) in previous
        .into_iter()
        .zip(["203.0.113.78:5353", "[2001:db8::78]:5353"])
    {
        let target: SocketAddr = target.parse().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        assert_eq!(
            socket
                .send_to(b"must-not-leak", target)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EPERM)
        );
        socket.send_to(b"udp-original-target", target).unwrap();
        let mut reply = [0; 64];
        let (length, source) = socket.recv_from(&mut reply).unwrap();
        assert_eq!(source, target);
        assert_eq!(&reply[..length], b"UDP-original-target");
    }
}
