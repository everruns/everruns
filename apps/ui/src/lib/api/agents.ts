// Agent API functions (M2)
// All routes are org-scoped: /v1/orgs/{org}/agents/...

import { api } from "./client";
import type {
  Agent,
  AgentPreviewResponse,
  CreateAgentRequest,
  PreviewAgentRequest,
  UpdateAgentRequest,
  ListResponse,
} from "./types";

export async function createAgent(
  org: string,
  request: CreateAgentRequest
): Promise<Agent> {
  const response = await api.post<Agent>(`/v1/orgs/${org}/agents`, request);
  return response.data;
}

export async function listAgents(org: string): Promise<Agent[]> {
  const response = await api.get<ListResponse<Agent>>(`/v1/orgs/${org}/agents`);
  return response.data.data;
}

export async function getAgent(org: string, agentId: string): Promise<Agent> {
  const response = await api.get<Agent>(`/v1/orgs/${org}/agents/${agentId}`);
  return response.data;
}

export async function updateAgent(
  org: string,
  agentId: string,
  request: UpdateAgentRequest
): Promise<Agent> {
  const response = await api.patch<Agent>(
    `/v1/orgs/${org}/agents/${agentId}`,
    request
  );
  return response.data;
}

export async function deleteAgent(org: string, agentId: string): Promise<void> {
  await api.delete(`/v1/orgs/${org}/agents/${agentId}`);
}

export async function exportAgent(org: string, agentId: string): Promise<string> {
  // Use fetch directly since we need to return text, not JSON
  const response = await fetch(`/api/v1/orgs/${org}/agents/${agentId}/export`, {
    credentials: "include",
  });
  if (!response.ok) {
    throw new Error(`Export failed: ${response.statusText}`);
  }
  return response.text();
}

export async function importAgent(org: string, markdown: string): Promise<Agent> {
  // Use fetch directly since we need to send text/markdown content type
  const response = await fetch(`/api/v1/orgs/${org}/agents/import`, {
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

export async function previewAgent(
  org: string,
  request: PreviewAgentRequest
): Promise<AgentPreviewResponse> {
  const response = await api.post<AgentPreviewResponse>(
    `/v1/orgs/${org}/agents/preview`,
    request
  );
  return response.data;
}
