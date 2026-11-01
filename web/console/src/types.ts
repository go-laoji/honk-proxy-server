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
}
