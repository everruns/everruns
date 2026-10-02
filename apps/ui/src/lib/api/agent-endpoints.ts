import { api } from "./client";
import type { AgentVersionPolicy, AgentEndpoint, EndpointTransport } from "./types";

export interface CreateAgentEndpointRequest {
  channel_type: EndpointTransport;
  channel_config: unknown;
  enabled: boolean;
  agent_version_policy?: AgentVersionPolicy;
  agent_version_id?: string;
}

export interface UpdateAgentEndpointRequest {
  channel_config?: unknown;
  enabled?: boolean;
  /** `pinned` needs `agent_version_id`; `default`/`latest` clear the pin. */
  agent_version_policy?: AgentVersionPolicy;
  agent_version_id?: string;
}

export interface TriggerAgentEndpointResult {
  session_id: string;
  created_session: boolean;
}
export interface SlackManifest {
  manifest_yaml: string;
  create_url: string;
}

export async function listAgentEndpoints(agentId: string): Promise<AgentEndpoint[]> {
  const response = await api.get<AgentEndpoint[]>(`/v1/agents/${agentId}/endpoints`);
  return response.data;
}

export async function getAgentEndpoint(
  agentId: string,
  endpointId: string,
): Promise<AgentEndpoint> {
  const response = await api.get<AgentEndpoint>(`/v1/agents/${agentId}/endpoints/${endpointId}`);
  return response.data;
}

export async function createAgentEndpoint(
  agentId: string,
  request: CreateAgentEndpointRequest,
): Promise<AgentEndpoint> {
  const response = await api.post<AgentEndpoint>(`/v1/agents/${agentId}/endpoints`, request);
  return response.data;
}

export async function updateAgentEndpoint(
  agentId: string,
  endpointId: string,
  request: UpdateAgentEndpointRequest,
): Promise<AgentEndpoint> {
  const response = await api.patch<AgentEndpoint>(
    `/v1/agents/${agentId}/endpoints/${endpointId}`,
    request,
  );
  return response.data;
}

export async function getSlackEndpointManifest(endpointId: string): Promise<SlackManifest> {
  const response = await api.get<SlackManifest>(`/v1/e/${endpointId}/slack/manifest`);
  return response.data;
}

export async function deleteAgentEndpoint(agentId: string, endpointId: string): Promise<void> {
  await api.delete(`/v1/agents/${agentId}/endpoints/${endpointId}`);
}

export async function publishAgentEndpoint(
  agentId: string,
  endpointId: string,
): Promise<AgentEndpoint> {
  const response = await api.post<AgentEndpoint>(
    `/v1/agents/${agentId}/endpoints/${endpointId}/publish`,
  );
  return response.data;
}

export async function unpublishAgentEndpoint(
  agentId: string,
  endpointId: string,
): Promise<AgentEndpoint> {
  const response = await api.post<AgentEndpoint>(
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

export interface SlackInstallCapability {
  supported: boolean;
  connected: boolean;
  reconnect_required: boolean;
  can_manage: boolean;
}

export async function getSlackInstallCapability(): Promise<SlackInstallCapability> {
  const response = await api.get<SlackInstallCapability>("/v1/slack/install");
  return response.data;
}

/** A Slack workspace the organization has connected. Never carries a token. */
export interface SlackWorkspace {
  id: string;
  /** `null` only briefly, for a connection made before workspaces were recorded. */
  team_id: string | null;
  team_name: string | null;
  status: "connected" | "reconnect_required";
  connected_at: string;
}

export async function listSlackWorkspaces(): Promise<SlackWorkspace[]> {
  const response = await api.get<SlackWorkspace[]>("/v1/slack/workspaces");
  return response.data;
}

/** Connects whichever workspace the refresh token belongs to. Admin only. */
export async function connectSlackWorkspace(refreshToken: string): Promise<SlackWorkspace> {
  const response = await api.post<SlackWorkspace>("/v1/slack/workspaces", {
    refresh_token: refreshToken,
  });
  return response.data;
}

export async function testSlackWorkspace(id: string): Promise<void> {
  await api.post(`/v1/slack/workspaces/${id}/test`);
}

export async function disconnectSlackWorkspace(id: string): Promise<void> {
  await api.delete(`/v1/slack/workspaces/${id}`);
}

/**
 * Start the one-click Slack install for an endpoint (EVE-1069).
 *
 * Keyed on the endpoint's own public id rather than the agent, because the
 * route is the same `/v1/e/{endpoint}` family Slack itself calls back into.
 *
 * `teamId` picks which connected workspace the agent's app is created in; it
 * may be omitted while the organization has connected exactly one.
 *
 * Answers 501 when the deployment holds no Slack app configuration token —
 * the self-hosted steady state, where the manual fields are the supported
 * path rather than a fallback from a failure.
 */
export async function beginSlackInstall(
  endpointId: string,
  teamId?: string | null,
): Promise<BeginSlackInstallResult> {
  const response = await api.post<BeginSlackInstallResult>(
    `/v1/e/${endpointId}/slack/install`,
    teamId ? { team_id: teamId } : undefined,
  );
  return response.data;
}
