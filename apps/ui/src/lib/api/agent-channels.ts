import { api } from "./client";
import type { AgentChannel, ChannelType } from "./types";

export interface CreateAgentChannelRequest {
  channel_type: ChannelType;
  channel_config: unknown;
  enabled: boolean;
}

export interface UpdateAgentChannelRequest {
  channel_config?: unknown;
  enabled?: boolean;
}

export interface TriggerAgentChannelResult {
  session_id: string;
  created_session: boolean;
}
export interface SlackManifest {
  manifest_yaml: string;
  create_url: string;
}

export async function listAgentChannels(agentId: string): Promise<AgentChannel[]> {
  const response = await api.get<AgentChannel[]>(`/v1/agents/${agentId}/channels`);
  return response.data;
}

export async function getAgentChannel(agentId: string, channelId: string): Promise<AgentChannel> {
  const response = await api.get<AgentChannel>(`/v1/agents/${agentId}/channels/${channelId}`);
  return response.data;
}

export async function createAgentChannel(
  agentId: string,
  request: CreateAgentChannelRequest,
): Promise<AgentChannel> {
  const response = await api.post<AgentChannel>(`/v1/agents/${agentId}/channels`, request);
  return response.data;
}

export async function updateAgentChannel(
  agentId: string,
  channelId: string,
  request: UpdateAgentChannelRequest,
): Promise<AgentChannel> {
  const response = await api.patch<AgentChannel>(
    `/v1/agents/${agentId}/channels/${channelId}`,
    request,
  );
  return response.data;
}

export async function getSlackChannelManifest(channelId: string): Promise<SlackManifest> {
  const response = await api.get<SlackManifest>(`/v1/channels/${channelId}/slack/manifest`);
  return response.data;
}

export async function deleteAgentChannel(agentId: string, channelId: string): Promise<void> {
  await api.delete(`/v1/agents/${agentId}/channels/${channelId}`);
}

export async function publishAgentChannel(
  agentId: string,
  channelId: string,
): Promise<AgentChannel> {
  const response = await api.post<AgentChannel>(
    `/v1/agents/${agentId}/channels/${channelId}/publish`,
  );
  return response.data;
}

export async function unpublishAgentChannel(
  agentId: string,
  channelId: string,
): Promise<AgentChannel> {
  const response = await api.post<AgentChannel>(
    `/v1/agents/${agentId}/channels/${channelId}/unpublish`,
  );
  return response.data;
}

export async function triggerAgentChannel(
  agentId: string,
  channelId: string,
): Promise<TriggerAgentChannelResult> {
  const response = await api.post<TriggerAgentChannelResult>(
    `/v1/agents/${agentId}/channels/${channelId}/trigger`,
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
 * Start the one-click Slack install for a channel (EVE-1069).
 *
 * Keyed on the channel's own public id rather than the agent, because the
 * route is the same `/v1/channels/{channel}` family Slack itself calls back into.
 *
 * `teamId` picks which connected workspace the agent's app is created in; it
 * may be omitted while the organization has connected exactly one.
 *
 * Answers 501 when the deployment holds no Slack app configuration token —
 * the self-hosted steady state, where the manual fields are the supported
 * path rather than a fallback from a failure.
 */
export async function beginSlackInstall(
  channelId: string,
  teamId?: string | null,
): Promise<BeginSlackInstallResult> {
  const response = await api.post<BeginSlackInstallResult>(
    `/v1/channels/${channelId}/slack/install`,
    teamId ? { team_id: teamId } : undefined,
  );
  return response.data;
}
