//! Read-only local parsing of a downloaded public source; does not dial its proxies.
use hps_config::{ProviderConfig, ProviderKind};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("pass a downloaded HTML path")?;
    let bytes = std::fs::read(path)?;
    let config = ProviderConfig {
        id: "yun-free".into(),
        name: "云代理".into(),
        kind: ProviderKind::YunFree,
        endpoint: "http://www.ip3366.net/free/".into(),
        ttl_secs: Some(300),
        count: 100,
        ..ProviderConfig::default()
    };
    let result = hps_provider::parse(&config, &bytes, hps_provider::unix_now())?;
    println!(
        "parsed={} rejected={} expiry_basis={} health=unverified",
        result.nodes.len(),
        result.rejected,
        result.nodes[0].expiry_basis
    );
    Ok(())
}
