use super::*;
use hps_config::{ProxyConfig, ProxyProtocol, UpstreamConfig};
use hps_domain::{NodeImportInput, NodeImportPreview, NodeImportRow};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

impl Supervisor {
    pub async fn preview_nodes(
        self: &Arc<Self>,
        input: NodeImportInput,
    ) -> Result<NodeImportPreview, ApplicationError> {
        let permit = self
            .node_operations
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::NodeCapacity)?;
        let current = self.current();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            parse_import(&current, &input).map(|p| p.0)
        })
        .await
        .map_err(|_| ApplicationError::Worker)?
    }
    pub async fn import_nodes(
        self: &Arc<Self>,
        input: NodeImportInput,
    ) -> Result<ConfigActivationResult, ApplicationError> {
        let permit = self
            .node_operations
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApplicationError::NodeCapacity)?;
        self.tracked(move |service| async move {
            let _permit = permit;
            let current = service.current();
            let snapshot = current.clone();
            let ((preview, nodes), checksum) = tokio::task::spawn_blocking(move || {
                parse_import(&snapshot, &input).map(|p| (p, input.preview_checksum))
            })
            .await
            .map_err(|_| ApplicationError::Worker)??;
            if !preview.applicable || checksum.as_deref() != Some(preview.checksum.as_str()) {
                return Err(ApplicationError::Invalid);
            }
            let mut config = current.base_config.clone();
            config.proxy.upstreams.extend(nodes);
            let revision = service
                .create_revision(ConfigInput {
                    toml: toml::to_string(&config).map_err(|_| ApplicationError::Invalid)?,
                    expected_revision: current.revision,
                    expected_draft_version: None,
                    note: "Manual node import".into(),
                    use_draft_credentials: false,
                })
                .await?;
            service
                .activate(ConfigActivate {
                    revision: revision.id,
                    expected_revision: current.revision,
                    defer_until_restart: false,
                })
                .await
        })
        .await
    }
}
fn parse_import(
    current: &RuntimeSnapshot,
    input: &NodeImportInput,
) -> Result<(NodeImportPreview, Vec<UpstreamConfig>), ApplicationError> {
    if input.document.len() > 1024 * 1024
        || input.document.trim().is_empty()
        || input.selected_rows.len() > 1000
    {
        return Err(ApplicationError::Invalid);
    }
    if input.expected_revision != current.revision {
        return Err(StorageError::Conflict.into());
    }
    let protocol = match input.default_protocol.as_str() {
        "http" => ProxyProtocol::Http,
        "https" => ProxyProtocol::Https,
        "socks5" => ProxyProtocol::Socks5,
        _ => return Err(ApplicationError::Invalid),
    };
    let raw: Vec<(u32, Result<UpstreamConfig, ()>)> = match input.format.as_str() {
        "lines" => {
            if input.document.lines().count() > 2000 {
                return Err(ApplicationError::Invalid);
            }
            input
                .document
                .lines()
                .enumerate()
                .filter(|(_, s)| !s.trim().is_empty() && !s.trim().starts_with('#'))
                .map(|(index, line)| {
                    let result = hps_config::parse_upstream_address(line.trim(), protocol)
                        .map(|mut node| {
                            let hash = Sha256::digest(
                                serde_json::to_vec(&node).expect("node serialization"),
                            );
                            node.id = format!("manual-{:x}", hash)[..47].to_owned();
                            node
                        })
                        .map_err(|_| ());
                    (index as u32 + 1, result)
                })
                .collect()
        }
        "json" => {
            let values = serde_json::from_str::<Vec<serde_json::Value>>(&input.document)
                .map_err(|_| ApplicationError::Invalid)?;
            values
                .into_iter()
                .enumerate()
                .map(|(i, value)| {
                    (
                        i as u32 + 1,
                        serde_json::from_value::<UpstreamConfig>(value).map_err(|_| ()),
                    )
                })
                .collect()
        }
        _ => return Err(ApplicationError::Invalid),
    };
    if raw.is_empty() || raw.len() > 1000 {
        return Err(ApplicationError::Invalid);
    }
    let selected = input.selected_rows.iter().copied().collect::<HashSet<_>>();
    if selected.len() != input.selected_rows.len()
        || selected.iter().any(|r| !raw.iter().any(|(i, _)| i == r))
    {
        return Err(ApplicationError::Invalid);
    }
    let mut ids = current
        .config
        .proxy
        .upstreams
        .iter()
        .map(|n| n.id.clone())
        .collect::<HashSet<_>>();
    let mut rows = vec![];
    let mut nodes = vec![];
    for (row, parsed) in raw {
        let mut error = parsed.as_ref().err().map(|_| "invalid_proxy_entry");
        if let Ok(node) = &parsed {
            if (ProxyConfig {
                upstreams: vec![node.clone()],
                ..Default::default()
            })
            .validate()
            .is_err()
            {
                error = Some("invalid_proxy_configuration");
            } else if !ids.insert(node.id.clone()) {
                error = Some("duplicate_node_id");
            }
        }
        let valid = error.is_none();
        rows.push(NodeImportRow {
            row,
            id: parsed.as_ref().ok().map(|n| n.id.clone()),
            protocol: parsed
                .as_ref()
                .ok()
                .map(|n| format!("{:?}", n.protocol).to_lowercase()),
            host: parsed.as_ref().ok().map(|n| n.host.clone()),
            port: parsed.as_ref().ok().map(|n| n.port),
            has_credentials: parsed.as_ref().is_ok_and(|n| n.credentials.is_some()),
            valid,
            error_code: error.map(str::to_owned),
        });
        if valid && (selected.is_empty() || selected.contains(&row)) {
            nodes.push(parsed.unwrap());
        }
    }
    let valid_count = rows.iter().filter(|r| r.valid).count();
    let invalid_count = rows.len() - valid_count;
    let applicable = !nodes.is_empty()
        && if selected.is_empty() {
            invalid_count == 0
        } else {
            rows.iter()
                .filter(|r| selected.contains(&r.row))
                .all(|r| r.valid)
        };
    let checksum = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                input.document.as_str(),
                input.format.as_str(),
                input.default_protocol.as_str(),
                input.expected_revision,
                &input.selected_rows
            ))
            .expect("import checksum")
        )
    );
    Ok((
        NodeImportPreview {
            running_revision: current.revision,
            checksum,
            rows,
            valid_count,
            invalid_count,
            selected_count: nodes.len(),
            applicable,
        },
        nodes,
    ))
}
