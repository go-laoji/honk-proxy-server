export interface VersionInfo {
  service: string;
  version: string;
  api_version: string;
  web_build_id: string | null;
}
export interface ServiceStatus {
  instance_id: string;
  instance_name: string;
  management_ready: boolean;
  proxy_ready: boolean;
  engine_state: string;
  started_at_unix: number;
  uptime_seconds: number;
  platform: string;
  version: VersionInfo;
}
interface FeatureCapability {
  available: boolean;
  reason: string | null;
}
export interface Capabilities {
  management_api: boolean;
  requires_authentication: boolean;
  http_proxy: FeatureCapability;
  https_proxy: FeatureCapability;
  socks5_proxy: FeatureCapability;
  linux_ebpf: {
    platform_supported: boolean;
    compiled: boolean;
    configured: boolean;
    attached: boolean;
    ready: boolean;
    reason: string;
  };
}
export interface DaemonConfig {
  instance_name: string;
  management_listen: string;
  log_filter: string;
  management_tls?: { certificate: string; private_key: string } | null;
  retention?: {
    config_versions: number;
    activity_days: number;
    task_days: number;
  };
}

export interface ConfigRevision {
  id: number;
  parent_id: number | null;
  checksum: string;
  created_at: number;
  note: string;
  applied_at: number | null;
}
export interface ConfigWorkspace {
  format_version: number;
  desired_revision: number;
  running_revision: number;
  current_toml: string;
  desired_toml: string;
  draft: {
    version: number;
    base_revision: number;
    toml: string;
    updated_at: number;
  };
  revisions: ConfigRevision[];
  activations: {
    id: number;
    revision: number;
    previous_revision: number;
    state: string;
    created_at: number;
    error_code: string | null;
  }[];
  pending_restart: string[];
  storage_path: string | null;
  draining_generations: number;
  startup_overrides: string[];
  management_certificate: {
    not_before_unix: number;
    not_after_unix: number;
    valid_now: boolean;
  } | null;
}
export interface ConfigValidation {
  valid: boolean;
  diagnostics: {
    path: string;
    message: string;
    line: number | null;
    column: number | null;
  }[];
  changes: {
    path: string;
    before: unknown;
    after: unknown;
    apply_mode: string;
  }[];
  restart_required: string[];
}

export interface ResourceLimits {
  max_connections: number;
  bytes_per_second: number;
  buffer_bytes: number;
  handshake_timeout_ms: number;
  connect_timeout_ms: number;
  idle_timeout_ms: number;
  max_header_bytes: number;
  min_remaining_lifetime_secs: number;
}
export interface InboundStatus {
  id: string;
  protocol: "http" | "https" | "socks5";
  configured_listen: string;
  configured_enabled: boolean;
  listen: string | null;
  running: boolean;
  active_connections: number;
  max_connections: number;
  bytes_per_second: number;
  accepted_connections: number;
  rejected_global_limit: number;
  rejected_inbound_limit: number;
  failed_connections: number;
  idle_timeouts: number;
  certificate: {
    not_before_unix: number;
    not_after_unix: number;
    valid_now: boolean;
  } | null;
  last_error: string | null;
}
export interface ProxyStatus {
  inbounds: InboundStatus[];
  active_connections: number;
  eligible_upstreams: number;
  limits: ResourceLimits;
  shutting_down: boolean;
}

export interface ExtractionTask {
  id: number;
  provider_id: string;
  request_id: string;
  config_fingerprint: string;
  state:
    | "accepted"
    | "requesting"
    | "succeeded"
    | "failed"
    | "outcome_unknown"
    | "discarded";
  requested_count: number;
  started_at: number;
  finished_at: number | null;
  added: number;
  rejected: number;
  removed: number;
  quota: number | null;
  error_code: string | null;
  retry_after_secs: number | null;
}
export interface ProviderConfig {
  id: string;
  name: string;
  enabled: boolean;
  kind: "kuaidaili" | "yun_free" | "json" | "text";
  endpoint: string;
  protocol: "http" | "https" | "socks5";
  count: number;
  ttl_secs: number | null;
  min_interval_secs: number;
  query_valid_time: boolean;
  [field: string]: unknown;
}
export interface ProviderView {
  configuration: ProviderConfig;
  active_leases: number;
  latest_task: ExtractionTask | null;
  health_state: string;
}
export interface LeaseInfo {
  id: number;
  batch_id: number;
  provider_id: string;
  upstream_id: string;
  protocol: string;
  host: string;
  port: number;
  received_at: number;
  expires_at: number;
  expiry_basis: string;
  state: string;
  region: string | null;
}
export interface ProviderDetail {
  provider: ProviderView;
  tasks: ExtractionTask[];
  leases: LeaseInfo[];
}

export interface ProviderControllerStatus {
  provider_id: string;
  enabled: boolean;
  mode: "on_demand" | "maintain";
  min_nodes: number;
  target_nodes: number;
  max_nodes: number;
  current_inventory: number;
  projected_inventory: number;
  pending_health_nodes?: number;
  inflight_nodes: number;
  demand_active: boolean;
  refilling: boolean;
  decision: string;
  planned_count: number;
  day_started_at: number;
  resets_at: number;
  used_requests: number;
  request_budget: number;
  reserved_nodes: number;
  node_budget: number;
  next_allowed_at: number;
  failure_count: number;
  paused_reason: string | null;
  last_task_id: number | null;
  acknowledged_at: number | null;
}
export interface ControllerEvaluation {
  status: ProviderControllerStatus;
  task: ExtractionTask | null;
}

export type HealthVerdict =
  | "unknown"
  | "queued"
  | "probing"
  | "healthy"
  | "degraded"
  | "failed"
  | "unsupported"
  | "expired"
  | "skipped";
export interface ProbeProfile {
  transport: "tcp" | "udp";
  family: "ipv4" | "ipv6";
  enabled: boolean;
  url: string;
  tls_server_name: string | null;
  ca_certificate: string | null;
  expected_status: number | null;
}
export interface HealthConfig {
  enabled: boolean;
  profiles: ProbeProfile[];
  max_parallel: number;
  queue_capacity: number;
  timeout_ms: number;
  interval_secs: number;
  max_staleness_secs: number;
  min_probe_margin_secs: number;
  daily_probe_budget: number;
  retry_initial_secs: number;
  retry_max_secs: number;
  traffic_failure_threshold: number;
  max_header_bytes: number;
}
export interface ProbeResult {
  node_id: string;
  identity: string;
  profile_fingerprint: string;
  transport: "tcp" | "udp";
  family: "ipv4" | "ipv6";
  verdict: HealthVerdict;
  phase: string;
  error_code: string | null;
  endpoint_reachable: boolean;
  authenticated: boolean | null;
  tunnel_established: boolean;
  http_status: number | null;
  target_ok: boolean | null;
  latency_ms: number;
  checked_at: number;
}
export interface NodeHealth {
  node_id: string;
  provider_id: string | null;
  protocol: string;
  host: string;
  port: number;
  enabled: boolean;
  remaining_secs: number | null;
  eligible: boolean;
  last_traffic: {
    phase: string;
    error_code: string;
    invalidated: boolean;
    observed_at: number;
  } | null;
  results: ProbeResult[];
}
export interface HealthRun {
  id: number;
  node_id: string;
  identity: string;
  profile_fingerprint: string;
  state: string;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
  result: ProbeResult | null;
}
export interface HealthTask {
  id: number;
  request_id: string;
  trigger: string;
  state: string;
  created_at: number;
  total: number;
  completed: number;
  healthy: number;
  failed: number;
  skipped: number;
  merged: number;
  runs: HealthRun[];
}
export interface HealthOverview {
  running_revision: number;
  configuration: HealthConfig;
  nodes: NodeHealth[];
  total_nodes: number;
  next_after: string | null;
  queued: number;
  probing: number;
  day_started_at: number;
  resets_at: number;
  used_probes: number;
  budget: number;
  tasks: HealthTask[];
}
