// Capability API functions

import { api } from "./client";
import type {
  AgentCapability,
  Capability,
  CapabilityId,
  ListResponse,
  UpdateAgentCapabilitiesRequest,
} from "./types";

export async function listCapabilities(): Promise<Capability[]> {
  const response = await api.get<ListResponse<Capability>>("/v1/capabilities");
  return response.data.data;
}

export async function getCapability(
  capabilityId: CapabilityId
): Promise<Capability> {
  const response = await api.get<Capability>(
    `/v1/capabilities/${capabilityId}`
  );
  return response.data;
}

export async function getAgentCapabilities(
  agentId: string
): Promise<AgentCapability[]> {
  const response = await api.get<ListResponse<AgentCapability>>(
    `/v1/agents/${agentId}/capabilities`
  );
  return response.data.data;
}

export async function setAgentCapabilities(
  agentId: string,
  request: UpdateAgentCapabilitiesRequest
): Promise<AgentCapability[]> {
  const response = await api.put<ListResponse<AgentCapability>>(
    `/v1/agents/${agentId}/capabilities`,
    request
  );
  return response.data.data;
}
