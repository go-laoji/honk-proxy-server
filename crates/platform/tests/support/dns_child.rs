use hickory_proto::op::Message;
use hps_dns::wire;
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream, UdpSocket},
    path::Path,
    time::Duration,
};
fn query(name: &str, id: u16) -> Vec<u8> {
    let mut message = wire::query(name, "A").unwrap();
    message.metadata.id = id;
    message.to_vec().unwrap()
}
fn received(bytes: &[u8], id: u16) {
    let response = Message::from_vec(bytes).unwrap();
    assert_eq!(response.metadata.id, id);
    assert_eq!(
        wire::addresses(&response),
        vec!["203.0.113.9".parse::<std::net::IpAddr>().unwrap()]
    );
}
pub fn run() {
    let Ok(group) = std::env::var("HPS_DNS_CHILD_CGROUP") else {
        return;
    };
    let group = Path::new(&group).canonicalize().unwrap();
    assert!(
        group
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hps-dns-test-")
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(group.join("cgroup.procs"))
        .unwrap()
        .write_all(format!("{}\n", std::process::id()).as_bytes())
        .unwrap();
    for (family, peers, bind) in [
        ("ipv4", ["203.0.113.53:53", "198.51.100.53:53"], "0.0.0.0:0"),
        ("ipv6", ["[2001:db8::53]:53", "[2001:db8::54]:53"], "[::]:0"),
    ] {
        let peers = peers.map(|p| p.parse::<SocketAddr>().unwrap());
        let socket = UdpSocket::bind(bind).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket.connect(peers[0]).unwrap();
        assert_eq!(socket.peer_addr().unwrap(), peers[0]);
        socket
            .send(&query(&format!("connected-{family}.example"), 11))
            .unwrap();
        let mut buffer = vec![0; 65535];
        let (len, source) = socket.recv_from(&mut buffer).unwrap();
        assert_eq!(source, peers[0]);
        received(&buffer[..len], 11);
        let socket = UdpSocket::bind(bind).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .send_to(
                &query(&format!("reverse-one-{family}.example"), 21),
                peers[0],
            )
            .unwrap();
        socket
            .send_to(
                &query(&format!("reverse-two-{family}.example"), 22),
                peers[1],
            )
            .unwrap();
        let (len, source) = socket.recv_from(&mut buffer).unwrap();
        assert_eq!(source, peers[1]);
        received(&buffer[..len], 22);
        let (len, source) = socket.recv_from(&mut buffer).unwrap();
        assert_eq!(source, peers[0]);
        received(&buffer[..len], 21);
        let mut tcp = TcpStream::connect_timeout(&peers[0], Duration::from_secs(3)).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        tcp.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        assert_eq!(tcp.peer_addr().unwrap(), peers[0]);
        let bytes = query(&format!("tcp-{family}.example"), 31);
        tcp.write_all(&(bytes.len() as u16).to_be_bytes()).unwrap();
        tcp.write_all(&bytes).unwrap();
        let mut len = [0; 2];
        tcp.read_exact(&mut len).unwrap();
        let len = u16::from_be_bytes(len) as usize;
        tcp.read_exact(&mut buffer[..len]).unwrap();
        received(&buffer[..len], 31);
    }
}
