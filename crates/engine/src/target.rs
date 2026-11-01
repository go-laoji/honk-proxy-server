use crate::ProxyError;
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
}
impl Target {
    pub fn new(host: &str, port: u16) -> Result<Self, ProxyError> {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if port == 0 || !hps_config::valid_host(host) {
            return Err(ProxyError::InvalidTarget);
        }
        Ok(Self {
            host: host.into(),
            port,
        })
    }
    pub fn from_authority(authority: &str) -> Result<Self, ProxyError> {
        let authority: http::uri::Authority =
            authority.parse().map_err(|_| ProxyError::InvalidTarget)?;
        let port = authority.port_u16().ok_or(ProxyError::InvalidTarget)?;
        Self::new(authority.host(), port)
    }
    pub fn authority(&self) -> String {
        if self.host.parse::<IpAddr>().is_ok_and(|ip| ip.is_ipv6()) {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
    pub(crate) fn socks_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self.host.parse::<IpAddr>() {
            Ok(IpAddr::V4(ip)) => {
                out.push(1);
                out.extend(ip.octets());
            }
            Ok(IpAddr::V6(ip)) => {
                out.push(4);
                out.extend(ip.octets());
            }
            Err(_) => {
                out.push(3);
                out.push(self.host.len() as u8);
                out.extend(self.host.as_bytes());
            }
        }
        out.extend(self.port.to_be_bytes());
        out
    }
}
