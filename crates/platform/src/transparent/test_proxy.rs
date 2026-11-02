//! Controlled RFC1928 relay fixture; business payloads really reach the target.
use hps_engine::udp::{decode_datagram, encode_datagram};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
pub(super) struct Proxy {
    pub(super) address: SocketAddr,
    pub(super) associations: Arc<AtomicUsize>,
    pub(super) datagrams: Arc<AtomicUsize>,
    stop: CancellationToken,
    accept: Option<tokio::task::JoinHandle<()>>,
    workers: TaskTracker,
}
impl Proxy {
    pub(super) async fn bind() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let associations = Arc::new(AtomicUsize::new(0));
        let datagrams = Arc::new(AtomicUsize::new(0));
        let stop = CancellationToken::new();
        let workers = TaskTracker::new();
        let cancel = stop.clone();
        let tracker = workers.clone();
        let connected = associations.clone();
        let relayed = datagrams.clone();
        let accept = tokio::spawn(async move {
            loop {
                let (control, peer) = tokio::select! {biased;_=cancel.cancelled()=>return,result=listener.accept()=>result.unwrap()};
                let cancel = cancel.clone();
                let connected = connected.clone();
                let relayed = relayed.clone();
                tracker.spawn(async move{tokio::select!{biased;_=cancel.cancelled()=>{},_=proxy_session(control,peer,connected,relayed)=>{}}});
            }
        });
        Self {
            address,
            associations,
            datagrams,
            stop,
            accept: Some(accept),
            workers,
        }
    }
    pub(super) async fn shutdown(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.accept.take() {
            task.abort();
            let _ = task.await;
        }
        self.workers.close();
        tokio::time::timeout(Duration::from_secs(2), self.workers.wait())
            .await
            .unwrap();
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.accept {
            task.abort();
        }
    }
}
async fn proxy_session(
    mut control: TcpStream,
    peer: SocketAddr,
    connected: Arc<AtomicUsize>,
    relayed: Arc<AtomicUsize>,
) {
    let mut greeting = [0; 3];
    control.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 1, 0]);
    control.write_all(&[5, 0]).await.unwrap();
    let mut command = [0; 10];
    control.read_exact(&mut command).await.unwrap();
    assert_eq!(command, [5, 3, 0, 1, 0, 0, 0, 0, 0, 0]);
    let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut response = vec![5, 0, 0, 1, 127, 0, 0, 1];
    response.extend_from_slice(&relay.local_addr().unwrap().port().to_be_bytes());
    control.write_all(&response).await.unwrap();
    connected.fetch_add(1, Ordering::Relaxed);
    let mut packet = vec![0; 65536];
    let mut control_byte = [0; 1];
    loop {
        let (length, client) = tokio::select! {biased;_=control.read(&mut control_byte)=>return,result=relay.recv_from(&mut packet)=>result.unwrap()};
        assert_eq!(client.ip(), peer.ip());
        let (target, payload) = decode_datagram(&packet[..length]).unwrap();
        let destination = SocketAddr::new(target.host.parse().unwrap(), target.port);
        let outgoing = UdpSocket::bind(if destination.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .await
        .unwrap();
        outgoing.connect(destination).await.unwrap();
        outgoing.send(payload).await.unwrap();
        let mut reply = [0; 65536];
        let (length, source) = tokio::select! {biased;_=control.read(&mut control_byte)=>return,result=outgoing.recv_from(&mut reply)=>result.unwrap()};
        relayed.fetch_add(1, Ordering::Relaxed);
        let source =
            hps_engine::target::Target::new(&source.ip().to_string(), source.port()).unwrap();
        relay
            .send_to(&encode_datagram(&source, &reply[..length]).unwrap(), client)
            .await
            .unwrap();
    }
}
