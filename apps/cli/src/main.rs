use clap::{Parser, Subcommand, ValueEnum};
use hps_client::{ClientError, HpsClient};
use hps_config::DaemonConfig;
use std::{io::Write, path::PathBuf, process::ExitCode, time::Duration};
mod local;
mod tasks;

#[derive(Parser)]
#[command(
    name = "hps",
    version,
    about = "Manage a local or remote honk proxy daemon without a desktop"
)]
struct Args {
    #[arg(long, global = true, env = "HPS_ENDPOINT")]
    endpoint: Option<String>,
    #[arg(long, global = true, conflicts_with = "endpoint")]
    profile: Option<String>,
    #[arg(long = "ca-cert", global = true, env = "HPS_CA_CERT")]
    ca_certificate: Option<PathBuf>,
    #[arg(long, global = true, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=300))]
    timeout: u64,
    #[arg(long, global = true, value_enum, default_value_t = Output::Text)]
    output: Output,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Output {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Explicitly manage an OS service on this computer; never sent to a remote daemon.
    Service {
        #[arg(long, required = true)]
        local: bool,
        #[arg(long, default_value = "local")]
        id: String,
        #[command(subcommand)]
        command: local::ServiceCommand,
    },
    Profile {
        #[command(subcommand)]
        command: local::ProfileCommand,
    },
    Task {
        #[command(subcommand)]
        command: tasks::TaskCommand,
    },
    Event {
        #[command(subcommand)]
        command: tasks::EventCommand,
    },
    Status,
    Version,
    Capabilities,
    Inbound {
        #[command(subcommand)]
        command: InboundCommand,
    },
    Health {
        #[command(subcommand)]
        command: HealthCommand,
    },
    Dns {
        #[command(subcommand)]
        command: DnsCommand,
    },
    Group {
        #[command(subcommand)]
        command: GroupCommand,
    },
    Rule {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Node {
        #[command(subcommand)]
        command: NodeCommand,
    },
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    Linux {
        #[command(subcommand)]
        command: LinuxCommand,
    },
    Init {
        #[arg(long, default_value = "hps.toml")]
        config: PathBuf,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Ui {
        #[arg(long)]
        open: bool,
    },
}

#[derive(Subcommand)]
enum HealthCommand {
    Show {
        #[arg(long)]
        after: Option<String>,
        #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=200))]
        limit: u32,
    },
    Probe {
        #[arg(long = "node")]
        nodes: Vec<String>,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long)]
        wait: bool,
    },
    Task {
        id: i64,
    },
    Save {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
    },
}
#[derive(Subcommand)]
enum DnsCommand {
    Show {
        #[arg(long)]
        after: Option<String>,
        #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=200))]
        limit: u32,
    },
    Resolve {
        name: String,
        #[arg(long, default_value = "A")]
        query_type: String,
        #[arg(long,value_enum,default_value_t=DnsPurpose::Business)]
        purpose: DnsPurpose,
    },
    Context {
        ip: std::net::IpAddr,
    },
    Clear {
        #[arg(long)]
        domain: Option<String>,
    },
    Save {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
    },
}
#[derive(clap::Args)]
struct NodeActionArgs {
    id: String,
    #[arg(long)]
    expected_revision: i64,
    #[arg(long)]
    expected_control_version: i64,
}
#[derive(clap::Args)]
struct ConnectionListArgs {
    #[arg(long)]
    snapshot: Option<String>,
    #[arg(long)]
    expected_epoch: Option<String>,
    #[arg(long)]
    id: Option<String>,
    #[arg(long)]
    source: Option<String>,
    #[arg(long)]
    source_port: Option<u16>,
    #[arg(long)]
    destination: Option<String>,
    #[arg(long)]
    destination_port: Option<u16>,
    #[arg(long)]
    inbound: Option<String>,
    #[arg(long)]
    protocol: Option<String>,
    #[arg(long)]
    transport: Option<String>,
    #[arg(long)]
    origin: Option<String>,
    #[arg(long)]
    node: Option<String>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    group: Option<String>,
    #[arg(long)]
    rule: Option<String>,
    #[arg(long)]
    state: Option<String>,
    #[arg(long)]
    error: Option<String>,
    #[arg(long)]
    created_after_unix_ms: Option<u64>,
    #[arg(long)]
    sort: Option<String>,
    #[arg(long, default_value_t = 0)]
    offset: u32,
    #[arg(long, default_value_t = 50)]
    limit: u32,
}
impl ConnectionListArgs {
    fn query(&self) -> hps_domain::ConnectionQuery {
        hps_domain::ConnectionQuery {
            snapshot: self.snapshot.clone(),
            expected_epoch: self.expected_epoch.clone(),
            id: self.id.clone(),
            source: self.source.clone(),
            source_port: self.source_port,
            destination: self.destination.clone(),
            destination_port: self.destination_port,
            inbound: self.inbound.clone(),
            protocol: self.protocol.clone(),
            transport: self.transport.clone(),
            origin: self.origin.clone(),
            node: self.node.clone(),
            provider: self.provider.clone(),
            group: self.group.clone(),
            rule: self.rule.clone(),
            state: self.state.clone(),
            error: self.error.clone(),
            created_after_unix_ms: self.created_after_unix_ms,
            sort: self.sort.clone(),
            offset: self.offset,
            limit: Some(self.limit),
        }
    }
}
#[derive(Subcommand)]
enum ConnectionCommand {
    List(Box<ConnectionListArgs>),
    Summary,
    Show {
        id: String,
    },
    Disconnect {
        id: String,
        #[arg(long)]
        expected_epoch: String,
        /// Confirm this exact connection; an HTTP exchange may close its inbound keep-alive socket.
        #[arg(long)]
        confirm: bool,
    },
    DisconnectPreview {
        #[arg(long)]
        snapshot: String,
        #[arg(long)]
        expected_epoch: String,
        #[arg(long, value_delimiter = ',')]
        ids: Vec<String>,
    },
    DisconnectConfirm {
        #[arg(long)]
        token: String,
        #[arg(long)]
        expected_epoch: String,
        #[arg(long)]
        confirm: bool,
    },
}
#[derive(clap::Args)]
struct NodeImportArgs {
    path: PathBuf,
    #[arg(long,default_value="lines",value_parser=["lines","json"])]
    format: String,
    #[arg(long,default_value="http",value_parser=["http","https","socks5"])]
    protocol: String,
    #[arg(long)]
    expected_revision: i64,
    #[arg(long, value_delimiter = ',')]
    rows: Vec<u32>,
}
#[derive(Subcommand)]
enum NodeCommand {
    List {
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long)]
        protocol: Option<String>,
        #[arg(long)]
        capability: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        remaining_max_secs: Option<u64>,
        #[arg(long)]
        sort: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    Show {
        id: String,
    },
    Enable(NodeActionArgs),
    Disable(NodeActionArgs),
    Drain(NodeActionArgs),
    /// Recoverably suppress only this node; other leases and active sockets remain.
    Delete(NodeActionArgs),
    ImportPreview(NodeImportArgs),
    ImportApply {
        #[command(flatten)]
        input: NodeImportArgs,
        #[arg(long)]
        checksum: String,
    },
}
#[derive(Subcommand)]
enum GroupCommand {
    Show,
    Simulate {
        #[arg(long)]
        group: Option<String>,
        #[arg(long)]
        inbound: Option<String>,
        #[arg(long)]
        source_ip: Option<std::net::IpAddr>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        udp: bool,
        #[arg(long)]
        after: Option<String>,
        #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=200))]
        limit: u32,
    },
    Save {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
    },
    Upsert {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
    },
    Delete {
        id: String,
        #[arg(long)]
        expected_revision: i64,
    },
    Pin {
        id: String,
        node: String,
        #[arg(long = "fallback")]
        fallback: Vec<String>,
        #[arg(long)]
        expected_revision: i64,
    },
    Unpin {
        id: String,
        #[arg(long)]
        expected_revision: i64,
    },
}
#[derive(Subcommand)]
enum RuleCommand {
    Show {
        #[arg(long)]
        revision: Option<i64>,
    },
    /// Save an unactivated routing policy candidate.
    Save {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        base_revision: Option<i64>,
    },
    /// Create or update a rule in a new unactivated candidate.
    Upsert {
        path: PathBuf,
        #[arg(long)]
        index: Option<usize>,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        base_revision: Option<i64>,
    },
    Delete {
        id: String,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        base_revision: Option<i64>,
    },
    Enable {
        id: String,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        base_revision: Option<i64>,
    },
    Disable {
        id: String,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        base_revision: Option<i64>,
    },
    Reorder {
        ids: Vec<String>,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        base_revision: Option<i64>,
    },
    /// Explicitly activate a saved rules-only configuration candidate.
    Apply {
        revision: i64,
        #[arg(long)]
        expected_revision: i64,
    },
    /// Read a JSON context and simulate without network or purchase side effects.
    Simulate {
        path: PathBuf,
        #[arg(long)]
        revision: Option<i64>,
    },
}
#[derive(Clone, Copy, clap::ValueEnum)]
enum DnsPurpose {
    Bootstrap,
    Provider,
    Health,
    Business,
}
impl From<DnsPurpose> for hps_net::ResolvePurpose {
    fn from(value: DnsPurpose) -> Self {
        match value {
            DnsPurpose::Bootstrap => Self::Bootstrap,
            DnsPurpose::Provider => Self::Provider,
            DnsPurpose::Health => Self::Health,
            DnsPurpose::Business => Self::Business,
        }
    }
}
#[derive(Subcommand)]
enum ConfigCommand {
    Show,
    Check {
        path: PathBuf,
    },
    Workspace,
    Validate {
        path: PathBuf,
        #[arg(long)]
        expected_revision: Option<i64>,
        #[arg(long)]
        draft_version: Option<i64>,
        #[arg(long, requires = "draft_version")]
        use_draft_credentials: bool,
    },
    SaveDraft {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        draft_version: i64,
        #[arg(long)]
        use_draft_credentials: bool,
    },
    Import {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long, default_value = "CLI import")]
        note: String,
        #[arg(long)]
        draft_version: Option<i64>,
        #[arg(long, requires = "draft_version")]
        use_draft_credentials: bool,
    },
    Activate {
        revision: i64,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        defer_until_restart: bool,
    },
    Rollback {
        revision: i64,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        defer_until_restart: bool,
    },
    Export {
        #[arg(long)]
        revision: Option<i64>,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum LinuxCommand {
    Preflight,
    Capture {
        #[command(subcommand)]
        command: CaptureCommand,
    },
    #[command(
        about = "Run a local Linux eBPF self-test in a new cgroup; does not use --endpoint or enable proxy capture"
    )]
    SelfTest {
        #[arg(long)]
        object: PathBuf,
        #[arg(long, default_value = "/sys/fs/cgroup")]
        cgroup_root: PathBuf,
    },
    #[command(hide = true)]
    SelfTestChild {
        #[arg(long)]
        cgroup: PathBuf,
        #[arg(long)]
        target: std::net::SocketAddr,
        #[arg(long)]
        expect_denied: bool,
    },
}
#[derive(Subcommand)]
enum CaptureCommand {
    Status,
    /// Save a candidate from a JSON capture object, or JSON null to disable.
    Save {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        expected_epoch: String,
    },
    Control {
        #[arg(value_enum)]
        action: CaptureAction,
        #[arg(long)]
        expected_revision: i64,
        #[arg(long)]
        expected_epoch: String,
    },
}
#[derive(Clone, clap::ValueEnum)]
enum CaptureAction {
    Start,
    Stop,
    Recheck,
}
#[derive(Subcommand)]
enum InboundCommand {
    List,
    Start { id: String },
    Stop { id: String },
}

#[derive(Subcommand)]
enum ProviderCommand {
    List,
    Controller {
        id: String,
    },
    Pause {
        id: String,
    },
    #[command(
        about = "Acknowledge an uncertain result and resume; preserves budgets and cooldowns"
    )]
    Resume {
        id: String,
    },
    Evaluate {
        id: String,
    },
    Show {
        id: String,
    },
    Save {
        path: PathBuf,
        #[arg(long)]
        expected_revision: i64,
    },
    Delete {
        id: String,
        #[arg(long)]
        expected_revision: i64,
    },
    Extract {
        id: String,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long)]
        wait: bool,
    },
    Task {
        id: i64,
    },
    Leases {
        id: String,
        #[arg(long)]
        before: Option<i64>,
    },
    Diagnose {
        id: String,
        #[arg(value_parser=["quota","whitelist"])]
        kind: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args).await {
        Ok(value) => {
            match args.output {
                Output::Json => println!("{}", value),
                Output::Text
                    if matches!(
                        args.command,
                        Command::Config {
                            command: ConfigCommand::Export { path: None, .. }
                        }
                    ) =>
                {
                    print!("{}", value["toml"].as_str().unwrap_or(""))
                }
                Output::Text => println!(
                    "{}",
                    serde_json::to_string_pretty(&value).expect("JSON value")
                ),
            }
            if value
                .get("state")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| {
                    ["failed", "outcome_unknown", "discarded", "interrupted"].contains(&s)
                })
                || value.get("valid").and_then(serde_json::Value::as_bool) == Some(false)
                || (matches!(
                    args.command,
                    Command::Health {
                        command: HealthCommand::Task { .. } | HealthCommand::Probe { .. }
                    }
                ) && value
                    .get("failed")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|n| n > 0))
            {
                ExitCode::from(5)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err((code, message)) => {
            eprintln!("{message}");
            ExitCode::from(code)
        }
    }
}

fn client_error(error: ClientError) -> (u8, String) {
    let code = match &error {
        ClientError::InvalidEndpoint
        | ClientError::InvalidCertificate
        | ClientError::InvalidConnectionId
        | ClientError::InvalidTaskId
        | ClientError::InvalidEventCursor => 2,
        ClientError::EventProtocol => 5,
        ClientError::Transport(e) if e.is_timeout() => 6,
        ClientError::Transport(_) => 3,
        ClientError::Api { status: 409, .. } => 7,
        ClientError::Api { .. } => 5,
    };
    (code, error.to_string())
}

async fn run(args: &Args) -> Result<serde_json::Value, (u8, String)> {
    // Local configuration commands do not require a reachable daemon.
    match &args.command {
        Command::Profile { command } => return local::profiles(command),
        Command::Service { id, command, .. } => {
            if args.endpoint.is_some() || args.profile.is_some() {
                return Err((2,"service commands act on this computer; omit --endpoint and --profile, pass --local explicitly".into()));
            }
            return local::service(id, command);
        }
        Command::Linux {
            command:
                LinuxCommand::SelfTest {
                    object,
                    cgroup_root,
                },
        } => {
            let executable = std::env::current_exe().map_err(|e| (2, e.to_string()))?;
            return serde_json::to_value(
                hps_platform::ebpf::self_test(object, cgroup_root, &executable)
                    .map_err(|e| (5, e))?,
            )
            .map_err(|e| (5, e.to_string()));
        }
        Command::Linux {
            command:
                LinuxCommand::SelfTestChild {
                    cgroup,
                    target,
                    expect_denied,
                },
        } => {
            hps_platform::ebpf::self_test_child(cgroup, *target, *expect_denied)
                .map_err(|e| (5, e))?;
            return Ok(serde_json::json!({"test_helper":true,"expected_denied":expect_denied}));
        }
        Command::Init { config } => {
            let body =
                toml::to_string_pretty(&DaemonConfig::default()).map_err(|e| (2, e.to_string()))?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(config)
                .map_err(|e| (2, e.to_string()))?;
            file.write_all(body.as_bytes())
                .map_err(|e| (2, e.to_string()))?;
            return Ok(serde_json::json!({"created": config}));
        }
        Command::Config {
            command: ConfigCommand::Check { path },
        } => {
            DaemonConfig::load(path).map_err(|e| (2, e.to_string()))?;
            return Ok(serde_json::json!({"valid": true, "path": path}));
        }
        _ => {}
    }
    let selected = if args.endpoint.is_none() {
        let profiles =
            hps_system::profiles::Store::new(&hps_system::state_dir().map_err(|e| (2, e))?)
                .load()
                .map_err(|e| (2, e))?;
        if let Some(id) = &args.profile {
            Some(
                profiles
                    .profiles
                    .into_iter()
                    .find(|p| &p.id == id)
                    .ok_or((2, "profile not found".into()))?,
            )
        } else {
            profiles.active().cloned()
        }
    } else {
        None
    };
    let address = args
        .endpoint
        .as_deref()
        .or_else(|| selected.as_ref().map(|p| p.endpoint.as_str()))
        .unwrap_or("http://127.0.0.1:17900");
    let ca_path = args
        .ca_certificate
        .as_ref()
        .or_else(|| selected.as_ref().and_then(|p| p.ca_certificate.as_ref()));
    let certificate = if let Some(path) = ca_path {
        let metadata = std::fs::metadata(path).map_err(|error| (2, error.to_string()))?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err((
                2,
                "CA bundle must be a regular file no larger than 1 MiB".into(),
            ));
        }
        Some(std::fs::read(path).map_err(|error| (2, error.to_string()))?)
    } else {
        None
    };
    let client = HpsClient::with_ca(
        address,
        Duration::from_secs(args.timeout),
        certificate.as_deref(),
    )
    .map_err(client_error)?;
    let value = match &args.command {
        Command::Task { command } => return tasks::run_task(&client, command).await,
        Command::Event { command } => {
            return tasks::run_events(&client, command, args.output).await;
        }
        Command::Status => serde_json::to_value(client.status().await.map_err(client_error)?),
        Command::Version => serde_json::to_value(client.version().await.map_err(client_error)?),
        Command::Capabilities => {
            serde_json::to_value(client.capabilities().await.map_err(client_error)?)
        }
        Command::Inbound { command } => match command {
            InboundCommand::List => {
                serde_json::to_value(client.inbounds().await.map_err(client_error)?)
            }
            InboundCommand::Start { id } => {
                serde_json::to_value(client.start_inbound(id).await.map_err(client_error)?)
            }
            InboundCommand::Stop { id } => {
                serde_json::to_value(client.stop_inbound(id).await.map_err(client_error)?)
            }
        },
        Command::Connection { command } => match command {
            ConnectionCommand::List(input) => serde_json::to_value(
                client
                    .connections(&input.query())
                    .await
                    .map_err(client_error)?,
            ),
            ConnectionCommand::Summary => {
                serde_json::to_value(client.connection_summary().await.map_err(client_error)?)
            }
            ConnectionCommand::Show { id } => {
                serde_json::to_value(client.connection(id).await.map_err(client_error)?)
            }
            ConnectionCommand::Disconnect {
                id,
                expected_epoch,
                confirm,
            } => {
                if !confirm {
                    return Err((
                        2,
                        "review this connection, then pass --confirm to disconnect the exact ID"
                            .into(),
                    ));
                }
                serde_json::to_value(
                    client
                        .disconnect_connection(&hps_domain::ConnectionDisconnectInput {
                            id: id.clone(),
                            expected_epoch: expected_epoch.clone(),
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
            ConnectionCommand::DisconnectPreview {
                snapshot,
                expected_epoch,
                ids,
            } => serde_json::to_value(
                client
                    .preview_disconnect_connections(&hps_domain::ConnectionDisconnectPreviewInput {
                        snapshot: snapshot.clone(),
                        expected_epoch: expected_epoch.clone(),
                        ids: ids.clone(),
                    })
                    .await
                    .map_err(client_error)?,
            ),
            ConnectionCommand::DisconnectConfirm {
                token,
                expected_epoch,
                confirm,
            } => {
                if !confirm {
                    return Err((
                        2,
                        "review the immutable disconnect preview, then pass --confirm".into(),
                    ));
                }
                serde_json::to_value(
                    client
                        .confirm_disconnect_connections(&hps_domain::ConnectionDisconnectConfirm {
                            token: token.clone(),
                            expected_epoch: expected_epoch.clone(),
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
        },
        Command::Node { command } => match command {
            NodeCommand::List {
                search,
                provider,
                group,
                protocol,
                capability,
                state,
                remaining_max_secs,
                sort,
                offset,
                limit,
            } => serde_json::to_value(
                client
                    .nodes(&hps_domain::NodeQuery {
                        id: None,
                        search: search.clone(),
                        provider: provider.clone(),
                        group: group.clone(),
                        protocol: protocol.clone(),
                        capability: capability.clone(),
                        state: state.clone(),
                        remaining_max_secs: *remaining_max_secs,
                        sort: sort.clone(),
                        offset: *offset,
                        limit: Some(*limit),
                    })
                    .await
                    .map_err(client_error)?,
            ),
            NodeCommand::Show { id } => {
                serde_json::to_value(client.node(id).await.map_err(client_error)?)
            }
            NodeCommand::Enable(input)
            | NodeCommand::Disable(input)
            | NodeCommand::Drain(input)
            | NodeCommand::Delete(input) => {
                let detail = client.node(&input.id).await.map_err(client_error)?;
                let node = detail
                    .nodes
                    .first()
                    .ok_or_else(|| (2, "node not found".into()))?;
                let action = match command {
                    NodeCommand::Enable(_) => "enable",
                    NodeCommand::Disable(_) => "disable",
                    NodeCommand::Drain(_) => "drain",
                    _ => "delete",
                };
                serde_json::to_value(
                    client
                        .node_action(&hps_domain::NodeActionInput {
                            id: input.id.clone(),
                            action: action.into(),
                            expected_revision: input.expected_revision,
                            expected_control_version: input.expected_control_version,
                            lease_id: node.lease_id,
                            batch_id: node.batch_id,
                            provider_id: node.provider_id.clone(),
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
            NodeCommand::ImportPreview(input) | NodeCommand::ImportApply { input, .. } => {
                let request = hps_domain::NodeImportInput {
                    document: read_policy_file(&input.path)?,
                    format: input.format.clone(),
                    default_protocol: input.protocol.clone(),
                    expected_revision: input.expected_revision,
                    preview_checksum: match command {
                        NodeCommand::ImportApply { checksum, .. } => Some(checksum.clone()),
                        _ => None,
                    },
                    selected_rows: input.rows.clone(),
                };
                match command {
                    NodeCommand::ImportPreview(_) => serde_json::to_value(
                        client.preview_nodes(&request).await.map_err(client_error)?,
                    ),
                    _ => serde_json::to_value(
                        client.import_nodes(&request).await.map_err(client_error)?,
                    ),
                }
            }
        },
        Command::Rule { command } => match command {
            RuleCommand::Show { revision } => {
                serde_json::to_value(client.rules(*revision).await.map_err(client_error)?)
            }
            RuleCommand::Save {
                path,
                expected_revision,
                base_revision,
            } => {
                let body = read_policy_file(path)?;
                let configuration = toml::from_str::<hps_config::RoutingConfig>(&body)
                    .map_err(|_| (2, "invalid routing policy TOML".into()))?;
                serde_json::to_value(
                    client
                        .save_rules(&hps_config::RulesSave {
                            configuration,
                            expected_revision: *expected_revision,
                            base_revision: *base_revision,
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
            RuleCommand::Upsert {
                path,
                index,
                expected_revision,
                base_revision,
            } => {
                let body = read_policy_file(path)?;
                let rule = toml::from_str::<hps_config::RuleConfig>(&body)
                    .map_err(|_| (2, "invalid rule TOML".into()))?;
                serde_json::to_value(
                    client
                        .upsert_rule(&hps_config::RuleUpsert {
                            rule,
                            index: *index,
                            expected_revision: *expected_revision,
                            base_revision: *base_revision,
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
            RuleCommand::Delete {
                id,
                expected_revision,
                base_revision,
            } => serde_json::to_value(
                client
                    .delete_rule(&hps_config::RuleDelete {
                        id: id.clone(),
                        expected_revision: *expected_revision,
                        base_revision: *base_revision,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            RuleCommand::Enable {
                id,
                expected_revision,
                base_revision,
            }
            | RuleCommand::Disable {
                id,
                expected_revision,
                base_revision,
            } => serde_json::to_value(
                client
                    .toggle_rule(&hps_config::RuleToggle {
                        id: id.clone(),
                        enabled: matches!(command, RuleCommand::Enable { .. }),
                        expected_revision: *expected_revision,
                        base_revision: *base_revision,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            RuleCommand::Reorder {
                ids,
                expected_revision,
                base_revision,
            } => serde_json::to_value(
                client
                    .reorder_rules(&hps_config::RuleOrder {
                        ids: ids.clone(),
                        expected_revision: *expected_revision,
                        base_revision: *base_revision,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            RuleCommand::Apply {
                revision,
                expected_revision,
            } => serde_json::to_value(
                client
                    .apply_rules(&hps_domain::ConfigActivate {
                        revision: *revision,
                        expected_revision: *expected_revision,
                        defer_until_restart: false,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            RuleCommand::Simulate { path, revision } => {
                let body = read_policy_file(path)?;
                let context = serde_json::from_str::<hps_domain::RoutingContext>(&body)
                    .map_err(|_| (2, "invalid routing context JSON".into()))?;
                serde_json::to_value(
                    client
                        .simulate_rules(&hps_domain::RulesSimulationInput {
                            context,
                            revision: *revision,
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
        },
        Command::Group { command } => match command {
            GroupCommand::Show => {
                serde_json::to_value(client.groups().await.map_err(client_error)?)
            }
            GroupCommand::Simulate {
                group,
                inbound,
                source_ip,
                host,
                port,
                udp,
                after,
                limit,
            } => serde_json::to_value(
                client
                    .simulate_group(&hps_domain::SelectionQuery {
                        node: None,
                        group: group.clone(),
                        inbound: inbound.clone(),
                        source_ip: *source_ip,
                        source_port: None,
                        destination_host: host.clone(),
                        destination_port: *port,
                        transport: if *udp {
                            hps_domain::SelectionTransport::Udp
                        } else {
                            hps_domain::SelectionTransport::Tcp
                        },
                        after: after.clone(),
                        limit: Some(*limit as usize),
                    })
                    .await
                    .map_err(client_error)?,
            ),
            GroupCommand::Save {
                path,
                expected_revision,
            } => {
                let body = read_policy_file(path)?;
                let configuration = toml::from_str::<hps_config::SelectionConfig>(&body)
                    .map_err(|_| (2, "invalid group policy TOML".into()))?;
                serde_json::to_value(
                    client
                        .save_groups(&hps_config::SelectionSave {
                            configuration,
                            expected_revision: *expected_revision,
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
            GroupCommand::Upsert {
                path,
                expected_revision,
            } => {
                let body = read_policy_file(path)?;
                let group = toml::from_str::<hps_config::GroupConfig>(&body)
                    .map_err(|_| (2, "invalid group TOML".into()))?;
                serde_json::to_value(
                    client
                        .upsert_group(&hps_config::GroupUpsert {
                            group,
                            expected_revision: *expected_revision,
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
            GroupCommand::Delete {
                id,
                expected_revision,
            } => serde_json::to_value(
                client
                    .delete_group(&hps_config::GroupDelete {
                        group_id: id.clone(),
                        expected_revision: *expected_revision,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            GroupCommand::Pin {
                id,
                node,
                fallback,
                expected_revision,
            } => serde_json::to_value(
                client
                    .pin_group(&hps_config::GroupPin {
                        group_id: id.clone(),
                        node_id: Some(node.clone()),
                        fallback_nodes: fallback.clone(),
                        expected_revision: *expected_revision,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            GroupCommand::Unpin {
                id,
                expected_revision,
            } => serde_json::to_value(
                client
                    .pin_group(&hps_config::GroupPin {
                        group_id: id.clone(),
                        node_id: None,
                        fallback_nodes: vec![],
                        expected_revision: *expected_revision,
                    })
                    .await
                    .map_err(client_error)?,
            ),
        },
        Command::Dns { command } => match command {
            DnsCommand::Show { after, limit } => serde_json::to_value(
                client
                    .dns(after.as_deref(), *limit)
                    .await
                    .map_err(client_error)?,
            ),
            DnsCommand::Resolve {
                name,
                query_type,
                purpose,
            } => serde_json::to_value(
                client
                    .resolve_dns(&hps_dns::LookupInput {
                        name: name.clone(),
                        query_type: query_type.to_ascii_uppercase(),
                        purpose: (*purpose).into(),
                    })
                    .await
                    .map_err(client_error)?,
            ),
            DnsCommand::Context { ip } => {
                serde_json::to_value(client.dns_context(*ip).await.map_err(client_error)?)
            }
            DnsCommand::Clear { domain } => serde_json::to_value(
                client
                    .clear_dns(&hps_dns::ClearInput {
                        domain: domain.clone(),
                    })
                    .await
                    .map_err(client_error)?,
            ),
            DnsCommand::Save {
                path,
                expected_revision,
            } => {
                let metadata = std::fs::metadata(path).map_err(|e| (2, e.to_string()))?;
                if !metadata.is_file() || metadata.len() > 512 * 1024 {
                    return Err((
                        2,
                        "DNS TOML must be a regular file no larger than 512 KiB".into(),
                    ));
                }
                let body = std::fs::read_to_string(path).map_err(|e| (2, e.to_string()))?;
                let configuration = toml::from_str::<hps_config::DnsConfig>(&body)
                    .map_err(|_| (2, "invalid DNS TOML; check syntax and fields".into()))?;
                serde_json::to_value(
                    client
                        .save_dns(&hps_config::DnsSave {
                            configuration,
                            expected_revision: *expected_revision,
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
        },
        Command::Health { command } => match command {
            HealthCommand::Show { after, limit } => serde_json::to_value(
                client
                    .health(after.as_deref(), *limit)
                    .await
                    .map_err(client_error)?,
            ),
            HealthCommand::Task { id } => {
                serde_json::to_value(client.health_task(*id).await.map_err(client_error)?)
            }
            HealthCommand::Save {
                path,
                expected_revision,
            } => {
                let metadata = std::fs::metadata(path).map_err(|e| (2, e.to_string()))?;
                if !metadata.is_file() || metadata.len() > 128 * 1024 {
                    return Err((
                        2,
                        "health TOML must be a regular file no larger than 128 KiB".into(),
                    ));
                }
                let body = std::fs::read_to_string(path).map_err(|e| (2, e.to_string()))?;
                let configuration: hps_config::HealthConfig = toml::from_str(&body)
                    .map_err(|_| (2, "invalid health TOML; check syntax and fields".into()))?;
                serde_json::to_value(
                    client
                        .save_health(&hps_config::HealthSave {
                            configuration,
                            expected_revision: *expected_revision,
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
            HealthCommand::Probe {
                nodes,
                request_id,
                wait,
            } => {
                let request_id = request_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                eprintln!(
                    "health request_id={request_id}; reuse this ID if acknowledgment is lost"
                );
                let mut task = client
                    .probe_health(&hps_domain::HealthProbeInput {
                        request_id,
                        node_ids: nodes.clone(),
                    })
                    .await
                    .map_err(client_error)?;
                if *wait {
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.timeout);
                    while task.state == "running" && tokio::time::Instant::now() < deadline {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        task = client.health_task(task.id).await.map_err(client_error)?;
                    }
                }
                serde_json::to_value(task)
            }
        },
        Command::Provider { command } => match command {
            ProviderCommand::Controller { id } => {
                serde_json::to_value(client.provider_controller(id).await.map_err(client_error)?)
            }
            ProviderCommand::Pause { id } => serde_json::to_value(
                client
                    .pause_provider(id, true)
                    .await
                    .map_err(client_error)?,
            ),
            ProviderCommand::Resume { id } => serde_json::to_value(
                client
                    .pause_provider(id, false)
                    .await
                    .map_err(client_error)?,
            ),
            ProviderCommand::Evaluate { id } => {
                serde_json::to_value(client.evaluate_provider(id).await.map_err(client_error)?)
            }
            ProviderCommand::List => {
                serde_json::to_value(client.providers().await.map_err(client_error)?)
            }
            ProviderCommand::Show { id } => {
                serde_json::to_value(client.provider(id).await.map_err(client_error)?)
            }
            ProviderCommand::Save {
                path,
                expected_revision,
            } => {
                let metadata = std::fs::metadata(path).map_err(|e| (2, e.to_string()))?;
                if !metadata.is_file() || metadata.len() > 512 * 1024 {
                    return Err((
                        2,
                        "provider TOML must be a regular file no larger than 512 KiB".into(),
                    ));
                }
                let body = std::fs::read_to_string(path).map_err(|e| (2, e.to_string()))?;
                let configuration: hps_config::ProviderConfig = toml::from_str(&body)
                    .map_err(|_| (2, "invalid provider TOML; check syntax and fields".into()))?;
                serde_json::to_value(
                    client
                        .save_provider(configuration, *expected_revision)
                        .await
                        .map_err(client_error)?,
                )
            }
            ProviderCommand::Delete {
                id,
                expected_revision,
            } => serde_json::to_value(
                client
                    .delete_provider(id, *expected_revision)
                    .await
                    .map_err(client_error)?,
            ),
            ProviderCommand::Extract {
                id,
                request_id,
                wait,
            } => {
                let key = request_id
                    .clone()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                eprintln!("extraction request_id={key}; reuse this ID if acknowledgment is lost");
                let mut task = client
                    .extract_provider(id, key)
                    .await
                    .map_err(client_error)?;
                if *wait {
                    let until = tokio::time::Instant::now() + Duration::from_secs(args.timeout);
                    while ["accepted", "requesting"].contains(&task.state.as_str()) {
                        if tokio::time::Instant::now() >= until {
                            eprintln!(
                                "extraction task {} continues in daemon; inspect it with provider task",
                                task.id
                            );
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        task = client.provider_task(task.id).await.map_err(client_error)?;
                    }
                }
                serde_json::to_value(task)
            }
            ProviderCommand::Task { id } => {
                serde_json::to_value(client.provider_task(*id).await.map_err(client_error)?)
            }
            ProviderCommand::Leases { id, before } => serde_json::to_value(
                client
                    .provider_leases(id, *before)
                    .await
                    .map_err(client_error)?,
            ),
            ProviderCommand::Diagnose { id, kind } => serde_json::to_value(
                client
                    .provider_diagnostic(id, kind)
                    .await
                    .map_err(client_error)?,
            ),
        },
        Command::Linux {
            command: LinuxCommand::Preflight,
        } => serde_json::to_value(client.linux_preflight().await.map_err(client_error)?),
        Command::Linux {
            command: LinuxCommand::Capture { command },
        } => match command {
            CaptureCommand::Save {
                path,
                expected_revision,
                expected_epoch,
            } => {
                let raw = read_policy_file(path)?;
                let configuration = serde_json::from_str(&raw).map_err(|e| (2, e.to_string()))?;
                serde_json::to_value(
                    client
                        .save_linux_capture(&hps_config::LinuxCaptureSave {
                            expected_revision: *expected_revision,
                            expected_epoch: expected_epoch.clone(),
                            configuration,
                        })
                        .await
                        .map_err(client_error)?,
                )
            }
            CaptureCommand::Status => {
                serde_json::to_value(client.linux_capture().await.map_err(client_error)?)
            }
            CaptureCommand::Control {
                action,
                expected_revision,
                expected_epoch,
            } => serde_json::to_value(
                client
                    .control_linux_capture(&hps_domain::LinuxCaptureRequest {
                        expected_revision: *expected_revision,
                        expected_epoch: expected_epoch.clone(),
                        action: match action {
                            CaptureAction::Start => hps_domain::LinuxCaptureAction::Start,
                            CaptureAction::Stop => hps_domain::LinuxCaptureAction::Stop,
                            CaptureAction::Recheck => hps_domain::LinuxCaptureAction::Recheck,
                        },
                    })
                    .await
                    .map_err(client_error)?,
            ),
        },
        Command::Config { command } => match command {
            ConfigCommand::Show => {
                serde_json::to_value(client.config().await.map_err(client_error)?)
            }
            ConfigCommand::Workspace => {
                serde_json::to_value(client.config_workspace().await.map_err(client_error)?)
            }
            ConfigCommand::Validate {
                path,
                expected_revision,
                draft_version,
                use_draft_credentials,
            } => {
                let expected = match expected_revision {
                    Some(expected) => *expected,
                    None => {
                        client
                            .config_workspace()
                            .await
                            .map_err(client_error)?
                            .desired_revision
                    }
                };
                serde_json::to_value(
                    client
                        .validate_config(&read_config_input(
                            path,
                            expected,
                            *draft_version,
                            String::new(),
                            *use_draft_credentials,
                        )?)
                        .await
                        .map_err(client_error)?,
                )
            }
            ConfigCommand::SaveDraft {
                path,
                expected_revision,
                draft_version,
                use_draft_credentials,
            } => serde_json::to_value(
                client
                    .save_draft(&read_config_input(
                        path,
                        *expected_revision,
                        Some(*draft_version),
                        String::new(),
                        *use_draft_credentials,
                    )?)
                    .await
                    .map_err(client_error)?,
            ),
            ConfigCommand::Import {
                path,
                expected_revision,
                note,
                draft_version,
                use_draft_credentials,
            } => serde_json::to_value(
                client
                    .create_revision(&read_config_input(
                        path,
                        *expected_revision,
                        *draft_version,
                        note.clone(),
                        *use_draft_credentials,
                    )?)
                    .await
                    .map_err(client_error)?,
            ),
            ConfigCommand::Activate {
                revision,
                expected_revision,
                defer_until_restart,
            } => serde_json::to_value(
                client
                    .activate_config(&hps_domain::ConfigActivate {
                        revision: *revision,
                        expected_revision: *expected_revision,
                        defer_until_restart: *defer_until_restart,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            ConfigCommand::Rollback {
                revision,
                expected_revision,
                defer_until_restart,
            } => serde_json::to_value(
                client
                    .rollback_config(&hps_domain::ConfigActivate {
                        revision: *revision,
                        expected_revision: *expected_revision,
                        defer_until_restart: *defer_until_restart,
                    })
                    .await
                    .map_err(client_error)?,
            ),
            ConfigCommand::Export { revision, path } => {
                let export = client
                    .export_config(*revision)
                    .await
                    .map_err(client_error)?;
                if let Some(path) = path {
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .map_err(|error| (2, error.to_string()))?;
                    file.write_all(export.toml.as_bytes())
                        .map_err(|error| (2, error.to_string()))?;
                    Ok(serde_json::json!({"created":path,"revision":export.revision}))
                } else {
                    serde_json::to_value(export)
                }
            }
            ConfigCommand::Check { .. } => unreachable!(),
        },
        Command::Ui { open } => {
            let url = client.console_url();
            if *open {
                open_browser(url.as_str()).map_err(|e| (3, e.to_string()))?;
            }
            Ok(serde_json::json!({"url": url.as_str()}))
        }
        Command::Init { .. }
        | Command::Profile { .. }
        | Command::Service { .. }
        | Command::Linux {
            command: LinuxCommand::SelfTest { .. } | LinuxCommand::SelfTestChild { .. },
        } => unreachable!(),
    };
    value.map_err(|e| (5, e.to_string()))
}

fn read_policy_file(path: &std::path::Path) -> Result<String, (u8, String)> {
    let metadata = std::fs::metadata(path).map_err(|e| (2, e.to_string()))?;
    if !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 {
        return Err((
            2,
            "policy file must be a regular file no larger than 8 MiB".into(),
        ));
    }
    std::fs::read_to_string(path).map_err(|e| (2, e.to_string()))
}
fn read_config_input(
    path: &std::path::Path,
    expected_revision: i64,
    expected_draft_version: Option<i64>,
    note: String,
    use_draft_credentials: bool,
) -> Result<hps_domain::ConfigInput, (u8, String)> {
    let body = std::fs::read_to_string(path).map_err(|error| (2, error.to_string()))?;
    if body.len() > 2 * 1024 * 1024 {
        return Err((2, "configuration file exceeds 2 MiB".into()));
    }
    Ok(hps_domain::ConfigInput {
        toml: body,
        expected_revision,
        expected_draft_version,
        note,
        use_draft_credentials,
    })
}

fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    if command.arg(url).status()?.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("browser could not be opened"))
    }
}
