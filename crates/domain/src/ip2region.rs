use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeLocationSource {
    ExitIp,
    EndpointIp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLocation {
    pub ip: IpAddr,
    pub source: NodeLocationSource,
    pub country: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub isp: Option<String>,
    pub country_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ip2RegionSource {
    Bundled,
    Updated,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Ip2RegionStatus {
    pub version: Option<String>,
    pub source: Option<Ip2RegionSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ip2RegionUpdate {
    pub database: Ip2RegionStatus,
    pub updated: bool,
}
