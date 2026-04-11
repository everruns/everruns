// Session resources API functions

import { api } from "./client";
import type { SessionResource } from "./types";

/** List all resources for a session (leased resources + subagents) */
export async function listSessionResources(sessionId: string): Promise<SessionResource[]> {
  const response = await api.get<SessionResource[]>(`/v1/sessions/${sessionId}/resources`);
  return response.data;
}
