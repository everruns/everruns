import { apiClient } from "./client";
import type { Run, CreateRunRequest } from "./types";

// Run CRUD
export async function getRuns(params?: {
  status?: string;
  agent_id?: string;
  limit?: number;
  offset?: number;
}): Promise<Run[]> {
  const searchParams = new URLSearchParams();
  if (params?.status) searchParams.set("status", params.status);
  if (params?.agent_id) searchParams.set("agent_id", params.agent_id);
  if (params?.limit) searchParams.set("limit", params.limit.toString());
  if (params?.offset) searchParams.set("offset", params.offset.toString());

  const query = searchParams.toString();
  return apiClient<Run[]>(`/v1/runs${query ? `?${query}` : ""}`);
}

export async function getRun(runId: string): Promise<Run> {
  return apiClient<Run>(`/v1/runs/${runId}`);
}

export async function createRun(data: CreateRunRequest): Promise<Run> {
  return apiClient<Run>("/v1/runs", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function cancelRun(runId: string): Promise<void> {
  return apiClient<void>(`/v1/runs/${runId}/cancel`, {
    method: "PATCH",
  });
}
