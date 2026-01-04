// Event API functions
// Events are SSE notifications for real-time updates

import { api, getDirectApiUrl } from "./client";
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

// SSE event stream URL builder
// Uses direct API URL (bypasses Next.js proxy) for browser EventSource connections
export function getEventStreamUrl(agentId: string, sessionId: string): string {
  return `${getDirectApiUrl()}/v1/agents/${agentId}/sessions/${sessionId}/sse`;
}
