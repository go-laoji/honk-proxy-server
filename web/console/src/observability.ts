export interface ObservationOverview {
  epoch: string;
  source: string;
  observed_at_unix_ms: number;
  requested_window_secs: number;
  sample_interval_secs: number;
  sample_limit: number;
  oldest_sample_unix_ms: number | null;
  resets_on_daemon_restart: boolean;
  wall_clock_changed: boolean;
  samples: {
    at_unix_ms: number;
    interval_ms: number;
    active: number;
    bytes_up_delta: number;
    bytes_down_delta: number;
    closed_delta: number;
    failed_delta: number;
    rejected_delta: number;
    dns: {
      hits: number;
      misses: number;
      merged: number;
      failures: number;
      cache_entries: number;
      cache_bytes: number;
      in_flight: number;
    };
  }[];
  statistics: {
    started_at_unix_ms: number;
    elapsed_ms: number;
    active: number;
    closed: number;
    failed: number;
    bytes_up: number;
    bytes_down: number;
  };
  rule_hits: {
    epoch: string;
    since_unix_ms: number;
    observed_at_unix_ms: number;
    counter_limit: number;
    untracked_hits: number;
    counters: {
      configuration_revision: number;
      rule_id: string | null;
      hits: number;
    }[];
  };
  correlations: {
    source: string;
    resolution_secs: number;
    retention_secs: number;
    partial_boundary_bucket: boolean;
    unrepresented_closed_flows: number;
    group_limit_per_bucket: number;
    total_groups: number;
    groups_truncated: boolean;
    groups: {
      source_ip: string | null;
      inbound: string | null;
      destination_host: string;
      destination_port: number;
      closed: number;
      failed: number;
      bytes_up: number;
      bytes_down: number;
    }[];
  };
}
