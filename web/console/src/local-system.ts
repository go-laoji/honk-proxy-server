export interface Profile {
  id: string;
  name: string;
  kind: "local" | "remote";
  endpoint: string;
  ca_certificate: string | null;
}
export interface Profiles {
  format_version: number;
  revision: number;
  selected: string | null;
  profiles: Profile[];
}
export interface LocalContext {
  network_changes_disabled: boolean;
  bundled_daemon: string | null;
  bundled_config: string | null;
  endpoint: string;
  profile_id: string | null;
  local: boolean;
  platform: string;
  service: {
    installed: boolean;
    manager: string;
    state: string;
    pid: number | null;
    detail: string;
    manifest: { endpoint: string; scope: "user" | "system" } | null;
  } | null;
  service_error: string | null;
  proxy: { supported: boolean; state: string; detail: string } | null;
  network_services: { id: string; name: string }[];
}
