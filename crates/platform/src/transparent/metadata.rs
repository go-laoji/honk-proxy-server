use super::Capture;
use hps_domain::{FlowOrigin, ProcessIdentity, RoutingContext, SelectionTransport};
use hps_ebpf_abi::{FlowKey, TP_GATEWAY, TP_LOCAL, TP_PROCESS_VERIFIED};
use std::net::{IpAddr, SocketAddr};

fn words(address: IpAddr) -> [u32; 4] {
    match address {
        IpAddr::V4(a) => [u32::from_ne_bytes(a.octets()), 0, 0, 0],
        IpAddr::V6(a) => {
            let bytes = a.octets();
            std::array::from_fn(|i| u32::from_ne_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()))
        }
    }
}
impl Capture {
    /// Called only with endpoints recovered from the actual accepted TCP socket
    /// or UDP ancillary data. No caller-supplied process metadata is accepted.
    pub fn routing_context(
        &self,
        source: SocketAddr,
        destination: SocketAddr,
        protocol: u32,
    ) -> Result<(i64, RoutingContext), String> {
        if source.is_ipv4() != destination.is_ipv4()
            || source.port() == 0
            || destination.port() == 0
            || !matches!(protocol, 6 | 17)
        {
            return Err("invalid captured endpoints or transport".into());
        }
        let key = FlowKey {
            family: if source.is_ipv4() { 4 } else { 6 },
            protocol,
            source: words(source.ip()),
            destination: words(destination.ip()),
            source_port: u32::from(source.port()),
            destination_port: u32::from(destination.port()),
            netns_cookie: self.netns_cookie()?,
        };
        let flow = self
            .flow(key)?
            .ok_or("captured tuple metadata is missing")?;
        if flow.revision == 0
            || flow.revision > i64::MAX as u64
            || flow.first_seen_ns == 0
            || flow.last_seen_ns < flow.first_seen_ns
            || flow.reserved != 0
            || flow.flags & !TP_PROCESS_VERIFIED != 0
        {
            return Err("invalid captured flow generation or ABI".into());
        }
        let (origin, process) = match flow.origin {
            TP_LOCAL
                if flow.flags & TP_PROCESS_VERIFIED != 0
                    && flow.process.pid > 0
                    && flow.process.thread_id > 0
                    && flow.process.socket_cookie > 0
                    && flow.process.cgroup_id > 0 =>
            {
                let end = flow
                    .process
                    .command
                    .iter()
                    .position(|b| *b == 0)
                    .unwrap_or(flow.process.command.len());
                (
                    FlowOrigin::LinuxLocal,
                    Some(ProcessIdentity {
                        pid: flow.process.pid,
                        command: String::from_utf8_lossy(&flow.process.command[..end]).into_owned(),
                        cgroup_id: flow.process.cgroup_id,
                    }),
                )
            }
            TP_GATEWAY => (FlowOrigin::Gateway, None),
            _ => return Err("captured local process metadata is missing or unverified".into()),
        };
        Ok((
            flow.revision as i64,
            RoutingContext {
                destination_host: destination.ip().to_string(),
                destination_port: destination.port(),
                transport: if protocol == 6 {
                    SelectionTransport::Tcp
                } else {
                    SelectionTransport::Udp
                },
                source_ip: Some(source.ip()),
                source_port: Some(source.port()),
                inbound: None,
                inbound_protocol: Some("transparent".into()),
                observed_domains: vec![],
                origin,
                process,
            },
        ))
    }
}
