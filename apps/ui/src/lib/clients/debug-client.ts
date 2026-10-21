import { api } from "./base-client";

export type DebugService = {
  status: "up" | "down";
  detail: Record<string, unknown>;
};

export type DatabaseStorage = {
  file_bytes: number | null;
  wal_bytes: number | null;
  shm_bytes: number | null;
  page_size: number | null;
  page_count: number | null;
  freelist_count: number | null;
  free_bytes: number | null;
  /** `bytes` when SQLite has dbstat; otherwise tables carry row counts instead. */
  per_table_mode: "bytes" | "rows";
  tables: { name: string; bytes: number | null; rows: number | null }[];
  /** The DuckDB metrics store; `error` instead of `stats` when it could not be read. */
  metrics: { stats: MetricsStoreStats | null; error: string | null };
  /** The Dragonfly ephemeral store; `error` instead of `stats` when it could not be reached. */
  dragonfly: { stats: DragonflyStats | null; error: string | null };
};

export type DragonflyStats = {
  version: string | null;
  uptime_secs: number | null;
  used_memory_bytes: number | null;
  peak_memory_bytes: number | null;
  /** 0 means no limit was configured. */
  max_memory_bytes: number | null;
  connected_clients: number | null;
  total_commands: number | null;
  keyspace_hits: number | null;
  keyspace_misses: number | null;
  expired_keys: number | null;
  evicted_keys: number | null;
  key_count: number;
  /** Second segment of `litepod:<group>:...` keys, largest first. */
  prefixes: { name: string; keys: number }[];
  /** The scan stopped early, so `prefixes` only covers part of the keys. */
  prefixes_truncated: boolean;
};

export type MetricsStoreStats = {
  file_bytes: number | null;
  wal_bytes: number | null;
  sample_count: number;
  /** Epoch milliseconds. */
  oldest_sample_at: number | null;
  newest_sample_at: number | null;
  container_count: number;
  sample_interval_secs: number;
  default_retention_days: number;
  /** Rows of the per-minute HTTP request rollups. */
  request_rows: number;
  /** Largest first; `bytes` counts checkpointed blocks only. */
  tables?: { name: string; bytes: number; rows: number }[];
};

export type DebugOverview = {
  caddy: DebugService;
  podman: DebugService;
};

export type BillingWebhookDelivery = {
  id: string;
  polar_event_id: string | null;
  event_type: string | null;
  request_headers_json: string;
  request_body: string;
  response_status: number | null;
  response_body: string | null;
  error_message: string | null;
  received_at: number;
  completed_at: number | null;
};

export type BillingDebugStatus = {
  configured: boolean;
  environment: "sandbox" | "production" | null;
  access_token_configured: boolean;
  webhook_secret_configured: boolean;
  events: {
    event_id: string;
    event_type: string;
    occurred_at: number;
    outcome: string;
    last_error: string | null;
  }[];
  deliveries: BillingWebhookDelivery[];
};

export async function getBillingDebugStatus() {
  return (await api<BillingDebugStatus>("/api/v1/debug/billing")).data!;
}

export async function getDatabaseStorage() {
  return (await api<DatabaseStorage>("/api/v1/debug/database")).data!;
}

export async function getDebugOverview() {
  return (await api<DebugOverview>("/api/v1/debug/overview")).data!;
}

// Caddy management moved to the per-server console — see `caddy-client.ts` and
// `/organizations/[id]/caddy`. Podman likewise — see `podman-console-client.ts`
// and `/organizations/[id]/containers`.
