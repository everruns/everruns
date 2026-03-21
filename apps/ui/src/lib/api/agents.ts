// Agent API functions (M2)
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api, throwApiError } from "./client";
import { createCrudApi } from "./crud";
import type {
  Agent,
  AgentPreviewResponse,
  CreateAgentRequest,
  PreviewAgentRequest,
  UpdateAgentRequest,
} from "./types";

export const agentsCrudApi = createCrudApi<Agent, CreateAgentRequest, UpdateAgentRequest>(
  "/v1/agents",
);

export const createAgent = agentsCrudApi.create;
export const listAgents = agentsCrudApi.list;
export const getAgent = agentsCrudApi.get;
export const updateAgent = agentsCrudApi.update;
export const deleteAgent = agentsCrudApi.delete;
export const destroyAgent = agentsCrudApi.destroy;

export async function exportAgent(agentId: string): Promise<string> {
  // Raw fetch needed: returns text/markdown, not JSON
  const response = await fetch(`/api/v1/agents/${agentId}/export`, {
    credentials: "include",
  });
  if (!response.ok) {
    await throwApiError(response);
  }
  return response.text();
}

export async function importAgent(markdown: string): Promise<Agent> {
  // Raw fetch needed: sends text/markdown Content-Type
  const response = await fetch("/api/v1/agents/import", {
    method: "POST",
    credentials: "include",
    headers: {
      "Content-Type": "text/markdown",
    },
    body: markdown,
  });
  if (!response.ok) {
    await throwApiError(response);
  }
  return response.json();
}

export async function copyAgent(agentId: string): Promise<Agent> {
  const response = await api.post<Agent>(`/v1/agents/${agentId}/copy`, {});
  return response.data;
}

export async function previewAgent(request: PreviewAgentRequest): Promise<AgentPreviewResponse> {
  const response = await api.post<AgentPreviewResponse>("/v1/agents/preview", request);
  return response.data;
}
