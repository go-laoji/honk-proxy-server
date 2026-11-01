//! Bounded metadata parsing shared by the TC program and host regression tests.
//! No payload copying, allocation, pointer casts or unbounded extension traversal.

pub trait PacketRead {
    fn packet_len(&self) -> usize;
    fn load<const N: usize>(&self, offset: usize) -> Option<[u8; N]>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketError {
    NotIp,
    NotTransport,
    Malformed,
    Fragmented,
    UnsupportedExtension,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkHeader {
    pub family: u32,
    pub source: [u32; 4],
    pub destination: [u32; 4],
    protocol: u8,
    network_offset: usize,
    transport_offset: usize,
    payload_end: usize,
    fragmented: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketHeader {
    pub family: u32,
    pub protocol: u32,
    pub source: [u32; 4],
    pub destination: [u32; 4],
    pub source_port: u16,
    pub destination_port: u16,
    pub tcp_flags: u8,
    pub tcp_sequence: u32,
    pub network_offset: usize,
    pub transport_offset: usize,
    pub payload_offset: usize,
    pub payload_end: usize,
}
#[inline(always)]
fn read<const N: usize>(
    packet: &impl PacketRead,
    offset: usize,
    end: usize,
) -> Result<[u8; N], PacketError> {
    if offset > end || N > end - offset {
        return Err(PacketError::Malformed);
    }
    packet.load(offset).ok_or(PacketError::Malformed)
}
#[inline(always)]
fn address(input: &[u8; 40], offset: usize) -> [u32; 4] {
    [
        u32::from_ne_bytes([
            input[offset],
            input[offset + 1],
            input[offset + 2],
            input[offset + 3],
        ]),
        u32::from_ne_bytes([
            input[offset + 4],
            input[offset + 5],
            input[offset + 6],
            input[offset + 7],
        ]),
        u32::from_ne_bytes([
            input[offset + 8],
            input[offset + 9],
            input[offset + 10],
            input[offset + 11],
        ]),
        u32::from_ne_bytes([
            input[offset + 12],
            input[offset + 13],
            input[offset + 14],
            input[offset + 15],
        ]),
    ]
}
/// Ethernet framing is present at TC ingress/egress, including loopback.
/// At most two visible VLAN tags and four IPv6 options/atomic-fragment headers.
#[inline(always)]
pub fn network_header(packet: &impl PacketRead) -> Result<NetworkHeader, PacketError> {
    let end = packet.packet_len();
    let ethernet = read::<14>(packet, 0, end)?;
    let mut ether_type = u16::from_be_bytes([ethernet[12], ethernet[13]]);
    let mut network = 14;
    for _ in 0..2 {
        if ether_type == 0x8100 || ether_type == 0x88a8 {
            let vlan = read::<4>(packet, network, end)?;
            ether_type = u16::from_be_bytes([vlan[2], vlan[3]]);
            network += 4;
        }
    }
    if matches!(ether_type, 0x8100 | 0x88a8) {
        return Err(PacketError::UnsupportedExtension);
    }
    let (family, source, destination, protocol, transport, ip_end, fragmented) = match ether_type {
        0x0800 => {
            let ip = read::<20>(packet, network, end)?;
            let header = usize::from(ip[0] & 15) * 4;
            let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
            if ip[0] >> 4 != 4 || header < 20 || total < header || total > end - network {
                return Err(PacketError::Malformed);
            }
            // MF or a nonzero offset: a partial transport must never be mistaken for a flow.
            let fragmented = u16::from_be_bytes([ip[6], ip[7]]) & 0x3fff != 0;
            let source = [
                u32::from_ne_bytes([ip[12], ip[13], ip[14], ip[15]]),
                0,
                0,
                0,
            ];
            let destination = [
                u32::from_ne_bytes([ip[16], ip[17], ip[18], ip[19]]),
                0,
                0,
                0,
            ];
            (
                4,
                source,
                destination,
                ip[9],
                network + header,
                network + total,
                fragmented,
            )
        }
        0x86dd => {
            let ip = read::<40>(packet, network, end)?;
            let payload = usize::from(u16::from_be_bytes([ip[4], ip[5]]));
            if ip[0] >> 4 != 6 || payload > end - network - 40 {
                return Err(PacketError::Malformed);
            }
            if payload == 0 {
                return Err(PacketError::UnsupportedExtension);
            } // no jumbograms
            (
                6,
                address(&ip, 8),
                address(&ip, 24),
                ip[6],
                network + 40,
                network + 40 + payload,
                false,
            )
        }
        _ => return Err(PacketError::NotIp),
    };
    Ok(NetworkHeader {
        family,
        source,
        destination,
        protocol,
        network_offset: network,
        transport_offset: transport,
        payload_end: ip_end,
        fragmented,
    })
}
#[inline(always)]
pub fn parse(packet: &impl PacketRead) -> Result<PacketHeader, PacketError> {
    transport_header(packet, &network_header(packet)?)
}
/// Call after CIDR bypass has been checked, so excluded destinations do not
/// become capture failures merely because their transport shape is unsupported.
#[inline(always)]
pub fn transport_header(
    packet: &impl PacketRead,
    header: &NetworkHeader,
) -> Result<PacketHeader, PacketError> {
    let NetworkHeader {
        family,
        source,
        destination,
        mut protocol,
        network_offset: network,
        transport_offset: mut transport,
        payload_end: ip_end,
        fragmented,
    } = *header;
    if fragmented {
        return Err(PacketError::Fragmented);
    }
    if family == 6 {
        for _ in 0..4 {
            match protocol {
                0 | 60 => {
                    let extension = read::<2>(packet, transport, ip_end)?;
                    let length = (usize::from(extension[1]) + 1) * 8;
                    if length > ip_end - transport || transport + length - network > 296 {
                        return Err(PacketError::UnsupportedExtension);
                    }
                    transport += length;
                    protocol = extension[0];
                }
                44 => {
                    let fragment = read::<8>(packet, transport, ip_end)?;
                    if u16::from_be_bytes([fragment[2], fragment[3]]) != 0 {
                        return Err(PacketError::Fragmented);
                    }
                    transport += 8;
                    protocol = fragment[0];
                }
                _ => break,
            }
        }
        if matches!(protocol, 0 | 43 | 44 | 50 | 51 | 60) {
            return Err(PacketError::UnsupportedExtension);
        }
    }
    let (source_port, destination_port, tcp_flags, tcp_sequence, payload_offset, payload_end) =
        match protocol {
            6 => {
                let tcp = read::<20>(packet, transport, ip_end)?;
                let header = usize::from(tcp[12] >> 4) * 4;
                if header < 20 || header > ip_end - transport {
                    return Err(PacketError::Malformed);
                }
                (
                    u16::from_be_bytes([tcp[0], tcp[1]]),
                    u16::from_be_bytes([tcp[2], tcp[3]]),
                    tcp[13],
                    u32::from_be_bytes([tcp[4], tcp[5], tcp[6], tcp[7]]),
                    transport + header,
                    ip_end,
                )
            }
            17 => {
                let udp = read::<8>(packet, transport, ip_end)?;
                let length = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
                if length < 8 || length > ip_end - transport {
                    return Err(PacketError::Malformed);
                }
                (
                    u16::from_be_bytes([udp[0], udp[1]]),
                    u16::from_be_bytes([udp[2], udp[3]]),
                    0,
                    0,
                    transport + 8,
                    transport + length,
                )
            }
            _ => return Err(PacketError::NotTransport),
        };
    if source_port == 0 || destination_port == 0 {
        return Err(PacketError::Malformed);
    }
    Ok(PacketHeader {
        family,
        protocol: u32::from(protocol),
        source,
        destination,
        source_port,
        destination_port,
        tcp_flags,
        tcp_sequence,
        network_offset: network,
        transport_offset: transport,
        payload_offset,
        payload_end,
    })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    use std::vec::Vec;
    impl PacketRead for Vec<u8> {
        fn packet_len(&self) -> usize {
            self.len()
        }
        fn load<const N: usize>(&self, offset: usize) -> Option<[u8; N]> {
            self.get(offset..offset.checked_add(N)?)?.try_into().ok()
        }
    }
    fn udp4() -> Vec<u8> {
        let mut bytes = vec![0; 14 + 20 + 8 + 3];
        bytes[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        bytes[14] = 0x45;
        bytes[16..18].copy_from_slice(&31u16.to_be_bytes());
        bytes[23] = 17;
        bytes[26..30].copy_from_slice(&[192, 0, 2, 1]);
        bytes[30..34].copy_from_slice(&[198, 51, 100, 2]);
        bytes[34..36].copy_from_slice(&1234u16.to_be_bytes());
        bytes[36..38].copy_from_slice(&443u16.to_be_bytes());
        bytes[38..40].copy_from_slice(&11u16.to_be_bytes());
        bytes
    }
    fn tcp6() -> Vec<u8> {
        let mut bytes = vec![0; 14 + 40 + 20];
        bytes[12..14].copy_from_slice(&0x86ddu16.to_be_bytes());
        bytes[14] = 0x60;
        bytes[18..20].copy_from_slice(&20u16.to_be_bytes());
        bytes[20] = 6;
        bytes[22..38].copy_from_slice(&[0x20, 1, 0xdb, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        bytes[38..54].copy_from_slice(&[0x20, 1, 0xdb, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);
        bytes[54..56].copy_from_slice(&1234u16.to_be_bytes());
        bytes[56..58].copy_from_slice(&443u16.to_be_bytes());
        bytes[58..62].copy_from_slice(&0x10203040u32.to_be_bytes());
        bytes[66] = 0x50;
        bytes[67] = 2;
        bytes
    }
    #[test]
    fn dual_stack_packets_keep_network_addresses_host_ports_and_exact_payload_bounds() {
        let mut packet = udp4();
        packet.extend_from_slice(&[0; 16]); // Ethernet padding is not UDP payload.
        let parsed = parse(&packet).unwrap();
        assert_eq!(
            (
                parsed.family,
                parsed.protocol,
                parsed.source_port,
                parsed.destination_port
            ),
            (4, 17, 1234, 443)
        );
        assert_eq!(parsed.destination[0].to_ne_bytes(), [198, 51, 100, 2]);
        assert_eq!((parsed.payload_offset, parsed.payload_end), (42, 45));
        let parsed = parse(&tcp6()).unwrap();
        assert_eq!(
            (
                parsed.family,
                parsed.protocol,
                parsed.tcp_flags,
                parsed.tcp_sequence
            ),
            (6, 6, 2, 0x10203040)
        );
        assert_eq!(parsed.destination[0].to_ne_bytes(), [0x20, 1, 0xdb, 8]);
    }
    #[test]
    fn options_vlan_and_atomic_ipv6_extensions_have_bounded_offsets() {
        let mut packet = udp4();
        packet.splice(34..34, [0; 4]);
        packet[14] = 0x46;
        packet[16..18].copy_from_slice(&35u16.to_be_bytes());
        packet.splice(14..14, [0, 1, 0x08, 0]);
        packet[12..14].copy_from_slice(&0x8100u16.to_be_bytes());
        assert_eq!(parse(&packet).unwrap().payload_offset, 50);
        let mut packet = tcp6();
        packet[20] = 0;
        packet[18..20].copy_from_slice(&36u16.to_be_bytes());
        packet.splice(54..54, [44, 0, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(parse(&packet).unwrap().transport_offset, 70);
        packet[64] = 0;
        packet[65] = 1;
        assert_eq!(parse(&packet), Err(PacketError::Fragmented));
    }
    #[test]
    fn truncated_fragmented_or_opaque_packets_never_become_an_invented_flow() {
        let original = udp4();
        for length in 0..original.len() {
            assert!(parse(&original[..length].to_vec()).is_err());
        }
        let mut packet = original.clone();
        packet[20] = 0x20;
        assert_eq!(parse(&packet), Err(PacketError::Fragmented));
        let mut packet = original;
        packet[38..40].copy_from_slice(&7u16.to_be_bytes());
        assert_eq!(parse(&packet), Err(PacketError::Malformed));
        let mut packet = tcp6();
        packet[20] = 43;
        assert_eq!(parse(&packet), Err(PacketError::UnsupportedExtension));
        let mut packet = tcp6();
        packet[66] = 0x10;
        assert_eq!(parse(&packet), Err(PacketError::Malformed));
    }
}
