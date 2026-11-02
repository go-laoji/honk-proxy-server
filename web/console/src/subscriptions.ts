import { getJson, postJson } from "./platform";
export type SubscriptionNode = {
  id: string;
  protocol: string;
  host: string;
  port: number;
  credentials: { username: string; password: string } | null;
  expires_at_unix: null;
};
export type SubscriptionConfig = {
  id: string;
  name: string;
  enabled: boolean;
  url: string;
  format: "uri" | "clash";
  timeout_ms: number;
  max_nodes: number;
  snapshot: {
    fetched_at_unix: number;
    origin: "remote_get" | "document_import";
    nodes: SubscriptionNode[];
  } | null;
};
export type SubscriptionsOverview = {
  running_revision: number;
  subscriptions: SubscriptionConfig[];
  expiry_semantics: string;
};
export const subscriptionApi = {
  list: (signal?: AbortSignal) =>
    getJson<SubscriptionsOverview>("api/v1/subscriptions", signal),
  save: (configuration: SubscriptionConfig, expected_revision: number) =>
    postJson(
      `api/v1/subscriptions/${encodeURIComponent(configuration.id)}/save`,
      { configuration, expected_revision },
    ),
  delete: (id: string, expected_revision: number) =>
    postJson(`api/v1/subscriptions/${encodeURIComponent(id)}/delete`, {
      expected_revision,
    }),
  refresh: (id: string, expected_revision: number, document: string | null) =>
    postJson(`api/v1/subscriptions/${encodeURIComponent(id)}/refresh`, {
      expected_revision,
      document,
    }),
};
