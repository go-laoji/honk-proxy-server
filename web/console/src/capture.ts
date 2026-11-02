export interface LinuxCaptureConfig {
  object: string;
  local_cgroup: string | null;
  lan_interfaces: string[];
  ipv4: boolean;
  ipv6: boolean;
  tcp: boolean;
  udp: boolean;
  bypass_cidrs: string[];
  bypass_ports: number[];
  capture_mark: number;
  bypass_mark: number;
  routing_table: number;
  rule_priority: number;
  tcp_port: number;
  udp_port: number;
  max_flows: number;
  max_pending: number;
  max_udp_sessions: number;
  udp_idle_secs: number;
  max_datagram_bytes: number;
  max_queued_datagrams: number;
  max_queued_bytes: number;
}
export interface CaptureStatus {
  platform: string;
  configured: boolean;
  state: string;
  ready: boolean;
  reason: string | null;
  scope: {
    local_cgroup: string | null;
    lan_interfaces: string[];
    ipv4: boolean;
    ipv6: boolean;
    tcp: boolean;
    udp: boolean;
  } | null;
  object_path: string | null;
  object_fingerprint: string | null;
  userspace_revision: number;
  kernel_revision: number | null;
  suspended: boolean;
  links_present: boolean | null;
  routes_present: boolean | null;
  listeners: { transport: string; address: string }[];
  program_ids: number[];
  map_ids: number[];
  generations: number[];
  flow_entries: number;
  active_owners: number;
  rejected_ingress: number;
  resources: Record<string, number> | null;
  hooks: {
    hook: string;
    captured_packets: number;
    rejected_packets: number;
    errors: number;
  }[];
  checked_at_unix_ms: number;
  retry_attempts: number;
  max_retry_attempts: number;
}
export interface CaptureOverview {
  epoch: string;
  running_revision: number;
  desired_revision: number;
  running_configuration: LinuxCaptureConfig | null;
  desired_configuration: LinuxCaptureConfig | null;
  status: CaptureStatus;
}
export const defaultCapture = (): LinuxCaptureConfig => ({
  object: "/opt/hps/hps-ebpf",
  local_cgroup: "/sys/fs/cgroup/hps-clients",
  lan_interfaces: [],
  ipv4: true,
  ipv6: true,
  tcp: true,
  udp: true,
  bypass_cidrs: [
    "0.0.0.0/8",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "224.0.0.0/4",
    "240.0.0.0/4",
    "::/128",
    "::1/128",
    "fe80::/10",
    "ff00::/8",
  ],
  bypass_ports: [],
  capture_mark: 0x48500000,
  bypass_mark: 0x48500001,
  routing_table: 10530,
  rule_priority: 31530,
  tcp_port: 18080,
  udp_port: 18080,
  max_flows: 8192,
  max_pending: 128,
  max_udp_sessions: 1024,
  udp_idle_secs: 60,
  max_datagram_bytes: 65507,
  max_queued_datagrams: 256,
  max_queued_bytes: 4194304,
});
