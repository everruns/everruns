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

export interface BeginSlackInstallResult {
  /** Send the operator here; Slack shows one consent screen, then redirects back. */
  authorize_url: string;
}

/**
 * Start the one-click Slack install for an endpoint (EVE-1069).
 *
 * Keyed on the endpoint's own public id rather than the agent, because the
 * route is the same `/v1/e/{endpoint}` family Slack itself calls back into.
 *
 * Answers 501 when the deployment holds no Slack app configuration token —
 * the self-hosted steady state, where the manual fields are the supported
 * path rather than a fallback from a failure.
 */
export async function beginSlackInstall(endpointId: string): Promise<BeginSlackInstallResult> {
  const response = await api.post<BeginSlackInstallResult>(`/v1/e/${endpointId}/slack/install`);
  return response.data;
}
