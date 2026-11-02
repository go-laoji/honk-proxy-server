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
