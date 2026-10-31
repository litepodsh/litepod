import { api } from "./base-client";

export type SecurityEventFlag =
  "new_country" | "impossible_travel" | "distributed_failures";

export type SecurityEvent = {
  id: string;
  action: string;
  user_id: string | null;
  user_email: string | null;
  user_name: string | null;
  actor_user_id: string | null;
  actor_email: string | null;
  actor_token_name: string | null;
  metadata: string | null;
  source_ip: string | null;
  user_agent: string | null;
  country: string | null;
  city: string | null;
  asn: string | null;
  flag: SecurityEventFlag | null;
  created_at: string;
};

export type SecurityEventFilter = {
  user_id?: string;
  ip?: string;
  action?: string;
  flagged?: boolean;
  before?: string;
  limit?: number;
};

export async function listSecurityEvents(
  filter: SecurityEventFilter = {},
): Promise<SecurityEvent[]> {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(filter)) {
    if (value !== undefined && value !== "" && value !== false)
      params.set(key, String(value));
  }
  const query = params.toString();
  const resp = await api<SecurityEvent[]>(
    `/api/v1/admin/security-events${query ? `?${query}` : ""}`,
  );
  return resp.data ?? [];
}
