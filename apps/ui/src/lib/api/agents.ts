import { apiClient } from "./client";
import type {
  Agent,
  AgentVersion,
  CreateAgentRequest,
  CreateAgentVersionRequest,
  UpdateAgentRequest,
} from "./types";

// Agent CRUD
export async function getAgents(): Promise<Agent[]> {
  return apiClient<Agent[]>("/v1/agents");
}

export async function getAgent(agentId: string): Promise<Agent> {
  return apiClient<Agent>(`/v1/agents/${agentId}`);
}

export async function createAgent(data: CreateAgentRequest): Promise<Agent> {
  return apiClient<Agent>("/v1/agents", {
    method: "POST",
    body: JSON.stringify(data),
  });
}

export async function updateAgent(
  agentId: string,
  data: UpdateAgentRequest
): Promise<Agent> {
  return apiClient<Agent>(`/v1/agents/${agentId}`, {
    method: "PATCH",
    body: JSON.stringify(data),
  });
}

// Agent Versions
export async function getAgentVersions(
  agentId: string
): Promise<AgentVersion[]> {
  return apiClient<AgentVersion[]>(`/v1/agents/${agentId}/versions`);
}

export async function getAgentVersion(
  agentId: string,
  version: number
): Promise<AgentVersion> {
  return apiClient<AgentVersion>(`/v1/agents/${agentId}/versions/${version}`);
}

export async function createAgentVersion(
  agentId: string,
  data: CreateAgentVersionRequest
): Promise<AgentVersion> {
  return apiClient<AgentVersion>(`/v1/agents/${agentId}/versions`, {
    method: "POST",
    body: JSON.stringify(data),
  });
}
