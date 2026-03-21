import { api } from "./client";
import type {
  AgentIdentity,
  CreateAgentIdentityRequest,
  ListResponse,
  UpdateAgentIdentityRequest,
} from "./types";

export async function listAgentIdentities(includeArchived = false): Promise<AgentIdentity[]> {
  const response = await api.get<ListResponse<AgentIdentity>>(
    `/v1/agent-identities?include_archived=${includeArchived}`,
  );
  return response.data.data;
}

export async function getAgentIdentity(identityId: string): Promise<AgentIdentity> {
  const response = await api.get<AgentIdentity>(`/v1/agent-identities/${identityId}`);
  return response.data;
}

export async function createAgentIdentity(
  request: CreateAgentIdentityRequest,
): Promise<AgentIdentity> {
  const response = await api.post<AgentIdentity>("/v1/agent-identities", request);
  return response.data;
}

export async function updateAgentIdentity(
  identityId: string,
  request: UpdateAgentIdentityRequest,
): Promise<AgentIdentity> {
  const response = await api.patch<AgentIdentity>(`/v1/agent-identities/${identityId}`, request);
  return response.data;
}

export async function deleteAgentIdentity(identityId: string): Promise<void> {
  await api.delete(`/v1/agent-identities/${identityId}`);
}

export async function destroyAgentIdentity(identityId: string): Promise<void> {
  await api.post(`/v1/agent-identities/${identityId}/delete`);
}
