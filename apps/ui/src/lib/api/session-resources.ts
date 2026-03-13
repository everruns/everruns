// Session leased-resources API functions

import { api } from "./client";
import type { LeasedResource } from "./types";

/** List lifecycle-managed resources for a session */
export async function listSessionResources(sessionId: string): Promise<LeasedResource[]> {
  const response = await api.get<LeasedResource[]>(`/v1/sessions/${sessionId}/resources`);
  return response.data;
}
