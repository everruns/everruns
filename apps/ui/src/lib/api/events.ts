// Event API functions
// Events are SSE notifications for real-time updates

import { api, getDirectBackendUrl } from "./client";
import type { Event, ListResponse } from "./types";

// List events for a session (polling alternative to SSE)
export async function listEvents(
  agentId: string,
  sessionId: string
): Promise<Event[]> {
  const response = await api.get<ListResponse<Event>>(
    `/v1/agents/${agentId}/sessions/${sessionId}/events`
  );
  return response.data.data;
}

// Get SSE URL for real-time event streaming
// Uses direct backend URL to bypass Next.js proxy (proxies buffer SSE)
// Uses since_id for incremental updates (UUID v7 monotonically increasing)
export function getSseUrl(agentId: string, sessionId: string, sinceId?: string): string {
  const baseUrl = getDirectBackendUrl();
  const params = sinceId ? `?since_id=${sinceId}` : "";
  return `${baseUrl}/v1/agents/${agentId}/sessions/${sessionId}/sse${params}`;
}
