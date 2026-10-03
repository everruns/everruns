import { api } from "./client";
import type { ListResponse } from "./types";

export interface AuditLogEntry {
  id: string;
  domain: string;
  action: string;
  actor_id: string | null;
  event_type: string;
  target_type: string | null;
  target_id: string | null;
  ip_address: string | null;
  metadata: Record<string, unknown>;
  created_at: string;
}

export interface ListAuditLogsOptions {
  limit?: number;
  before?: string;
  eventType?: string;
}

export async function listAuditLogs(
  orgId: string,
  options: ListAuditLogsOptions = {},
): Promise<AuditLogEntry[]> {
  const params = new URLSearchParams();
  if (options.limit !== undefined) params.set("limit", String(options.limit));
  if (options.before) params.set("before", options.before);
  if (options.eventType) params.set("event_type", options.eventType);
  const query = params.toString();
  const response = await api.get<ListResponse<AuditLogEntry>>(
    `/v1/orgs/${encodeURIComponent(orgId)}/audit-logs${query ? `?${query}` : ""}`,
  );
  return response.data.data;
}
