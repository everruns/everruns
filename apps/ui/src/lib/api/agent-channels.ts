import { api } from "./client";
import type { AppChannel, ChannelType } from "./types";

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

export async function listAgentChannels(agentId: string): Promise<AppChannel[]> {
  const response = await api.get<AppChannel[]>(`/v1/agents/${agentId}/channels`);
  return response.data;
}

export async function getAgentChannel(agentId: string, channelId: string): Promise<AppChannel> {
  const response = await api.get<AppChannel>(`/v1/agents/${agentId}/channels/${channelId}`);
  return response.data;
}

export async function createAgentChannel(
  agentId: string,
  request: CreateAgentChannelRequest,
): Promise<AppChannel> {
  const response = await api.post<AppChannel>(`/v1/agents/${agentId}/channels`, request);
  return response.data;
}

export async function updateAgentChannel(
  agentId: string,
  channelId: string,
  request: UpdateAgentChannelRequest,
): Promise<AppChannel> {
  const response = await api.patch<AppChannel>(
    `/v1/agents/${agentId}/channels/${channelId}`,
    request,
  );
  return response.data;
}

export async function getSlackChannelManifest(channelId: string): Promise<SlackManifest> {
  const response = await api.get<SlackManifest>(`/v1/e/${channelId}/slack/manifest`);
  return response.data;
}

export async function deleteAgentChannel(agentId: string, channelId: string): Promise<void> {
  await api.delete(`/v1/agents/${agentId}/channels/${channelId}`);
}

export async function publishAgentChannel(agentId: string, channelId: string): Promise<AppChannel> {
  const response = await api.post<AppChannel>(
    `/v1/agents/${agentId}/channels/${channelId}/publish`,
  );
  return response.data;
}

export async function unpublishAgentChannel(
  agentId: string,
  channelId: string,
): Promise<AppChannel> {
  const response = await api.post<AppChannel>(
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

/**
 * Start the one-click Slack install for a channel (EVE-1069).
 *
 * Keyed on the channel's own public id rather than the agent, because the
 * route is the same `/v1/e/{channel}` family Slack itself calls back into.
 *
 * Answers 501 when the deployment holds no Slack app configuration token —
 * the self-hosted steady state, where the manual fields are the supported
 * path rather than a fallback from a failure.
 */
export async function beginSlackInstall(channelId: string): Promise<BeginSlackInstallResult> {
  const response = await api.post<BeginSlackInstallResult>(`/v1/e/${channelId}/slack/install`);
  return response.data;
}
