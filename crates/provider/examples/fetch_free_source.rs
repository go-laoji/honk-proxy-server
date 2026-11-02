//! Read one public free-source page without dialing its advertised proxies.
use hps_config::{ProviderConfig, ProviderKind};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ProviderConfig {
        id: "yun-free".into(),
        name: "云代理".into(),
        enabled: true,
        kind: ProviderKind::YunFree,
        endpoint: "http://www.ip3366.net/free/".into(),
        params: std::collections::BTreeMap::from([("stype".into(), "1".into())]),
        ttl_secs: Some(300),
        count: 15,
        ..Default::default()
    };
    let result = hps_provider::ProviderClient::new(&config)?
        .fetch(&config, hps_provider::unix_now())
        .await?;
    println!(
        "fetched={} rejected={} expiry_basis={} health=unverified",
        result.nodes.len(),
        result.rejected,
        result.nodes[0].expiry_basis
    );
    Ok(())
}
