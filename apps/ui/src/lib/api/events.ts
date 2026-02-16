// Event API functions
// Events are SSE notifications for real-time updates
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api, getApiBaseUrl } from "./client";
import type { Event, ListResponse } from "./types";

// Default event types to exclude in UI contexts (streaming delta events are noise)
export const DEFAULT_EXCLUDED_EVENTS = ["output.message.delta", "reason.thinking.delta"];

// List events for a session (polling alternative to SSE)
// Optional exclude parameter to filter out event types (e.g., delta events)
export async function listEvents(sessionId: string, exclude?: string[]): Promise<Event[]> {
  const params = new URLSearchParams();
  if (exclude) {
    for (const type of exclude) {
      params.append("exclude", type);
    }
  }
  const queryString = params.toString();
  const response = await api.get<ListResponse<Event>>(
    `/v1/sessions/${sessionId}/events${queryString ? `?${queryString}` : ""}`,
  );
  return response.data.data;
}

// Get SSE URL for real-time event streaming
// Uses since_id for incremental updates (UUID v7 monotonically increasing)
// Optional exclude parameter to filter out event types (e.g., delta events)
// Note: Org is sent via everruns_org cookie (EventSource sends cookies automatically)
export function getSseUrl(sessionId: string, sinceId?: string, exclude?: string[]): string {
  const baseUrl = getApiBaseUrl();
  const params = new URLSearchParams();
  if (sinceId) {
    params.set("since_id", sinceId);
  }
  if (exclude) {
    for (const type of exclude) {
      params.append("exclude", type);
    }
  }
  const queryString = params.toString();
  return `${baseUrl}/v1/sessions/${sessionId}/sse${queryString ? `?${queryString}` : ""}`;
}
