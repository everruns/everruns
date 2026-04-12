// Session resource registry API functions

import { api } from "./client";
import type { SessionResourceEntry } from "./types";

/** List all resources registered in the session resource registry. */
export async function listSessionResources(sessionId: string): Promise<SessionResourceEntry[]> {
  const response = await api.get<SessionResourceEntry[]>(`/v1/sessions/${sessionId}/resources`);
  return response.data;
}
