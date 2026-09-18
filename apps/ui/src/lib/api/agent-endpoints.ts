import { api } from "./client";
import type { AppChannel, ChannelType } from "./types";

export interface CreateAgentEndpointRequest {
  channel_type: ChannelType;
  channel_config: unknown;
  enabled: boolean;
}

export interface UpdateAgentEndpointRequest {
  channel_config?: unknown;
  enabled?: boolean;
}

export interface TriggerAgentEndpointResult {
  session_id: string;
  created_session: boolean;
}

export async function listAgentEndpoints(agentId: string): Promise<AppChannel[]> {
  const response = await api.get<AppChannel[]>(`/v1/agents/${agentId}/endpoints`);
  return response.data;
}

export async function getAgentEndpoint(agentId: string, endpointId: string): Promise<AppChannel> {
  const response = await api.get<AppChannel>(`/v1/agents/${agentId}/endpoints/${endpointId}`);
  return response.data;
}

export async function createAgentEndpoint(
  agentId: string,
  request: CreateAgentEndpointRequest,
): Promise<AppChannel> {
  const response = await api.post<AppChannel>(`/v1/agents/${agentId}/endpoints`, request);
  return response.data;
}

export async function updateAgentEndpoint(
  agentId: string,
  endpointId: string,
  request: UpdateAgentEndpointRequest,
): Promise<AppChannel> {
  const response = await api.patch<AppChannel>(
    `/v1/agents/${agentId}/endpoints/${endpointId}`,
    request,
  );
  return response.data;
}

export async function deleteAgentEndpoint(agentId: string, endpointId: string): Promise<void> {
  await api.delete(`/v1/agents/${agentId}/endpoints/${endpointId}`);
}

export async function publishAgentEndpoint(
  agentId: string,
  endpointId: string,
): Promise<AppChannel> {
  const response = await api.post<AppChannel>(
    `/v1/agents/${agentId}/endpoints/${endpointId}/publish`,
  );
  return response.data;
}

export async function unpublishAgentEndpoint(
  agentId: string,
  endpointId: string,
): Promise<AppChannel> {
  const response = await api.post<AppChannel>(
    `/v1/agents/${agentId}/endpoints/${endpointId}/unpublish`,
  );
  return response.data;
}

export async function triggerAgentEndpoint(
  agentId: string,
  endpointId: string,
): Promise<TriggerAgentEndpointResult> {
  const response = await api.post<TriggerAgentEndpointResult>(
    `/v1/agents/${agentId}/endpoints/${endpointId}/trigger`,
  );
  return response.data;
}
