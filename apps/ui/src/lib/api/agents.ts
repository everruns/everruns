// Agent API functions (M2)

import { api } from "./client";
import type {
  Agent,
  CreateAgentRequest,
  UpdateAgentRequest,
  ListResponse,
} from "./types";

export async function createAgent(
  request: CreateAgentRequest
): Promise<Agent> {
  const response = await api.post<Agent>("/v1/agents", request);
  return response.data;
}

export async function listAgents(): Promise<Agent[]> {
  const response = await api.get<ListResponse<Agent>>("/v1/agents");
  return response.data.data;
}

export async function getAgent(agentId: string): Promise<Agent> {
  const response = await api.get<Agent>(`/v1/agents/${agentId}`);
  return response.data;
}

export async function updateAgent(
  agentId: string,
  request: UpdateAgentRequest
): Promise<Agent> {
  const response = await api.patch<Agent>(
    `/v1/agents/${agentId}`,
    request
  );
  return response.data;
}

export async function deleteAgent(agentId: string): Promise<void> {
  await api.delete(`/v1/agents/${agentId}`);
}

export async function exportAgent(agentId: string): Promise<string> {
  // Use fetch directly since we need to return text, not JSON
  const response = await fetch(`/api/v1/agents/${agentId}/export`, {
    credentials: "include",
  });
  if (!response.ok) {
    throw new Error(`Export failed: ${response.statusText}`);
  }
  return response.text();
}

export async function importAgent(markdown: string): Promise<Agent> {
  // Use fetch directly since we need to send text/markdown content type
  const response = await fetch("/api/v1/agents/import", {
    method: "POST",
    credentials: "include",
    headers: {
      "Content-Type": "text/markdown",
    },
    body: markdown,
  });
  if (!response.ok) {
    const error = await response.text();
    throw new Error(error || `Import failed: ${response.statusText}`);
  }
  return response.json();
}
