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
