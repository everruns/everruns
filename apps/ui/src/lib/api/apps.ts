// App API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import { createCrudApi } from "./crud";
import type {
  AddChannelRequest,
  App,
  AppChannel,
  AppEndpointAuthConfig,
  AppRunBucket,
  AppRunEvent,
  CreateAppRequest,
  InvocationSessionMode,
  UpdateAppRequest,
  UpdateChannelRequest,
} from "./types";

export interface AddA2aChannelRequest {
  session_mode?: InvocationSessionMode;
  message: string;
  agent_card_name?: string;
  agent_card_description?: string;
  auth?: AppEndpointAuthConfig;
  enabled?: boolean;
}

export interface A2aChannelKeyResponse {
  /**
   * Plaintext A2A API key. Returned exactly once — the server only stores a
   * SHA-256 hash. Display it to the operator and tell them to save it.
   */
  api_key: string;
  channel: AppChannel;
}

export const appsCrudApi = createCrudApi<App, CreateAppRequest, UpdateAppRequest>("/v1/apps");

export const getApps = appsCrudApi.list;
export const getApp = appsCrudApi.get;
export const createApp = appsCrudApi.create;
export const updateApp = appsCrudApi.update;
export const deleteApp = appsCrudApi.delete;
export const destroyApp = appsCrudApi.destroy;

export async function publishApp(appId: string): Promise<App> {
  const response = await api.post<App>(`/v1/apps/${appId}/publish`);
  return response.data;
}

export async function unpublishApp(appId: string): Promise<App> {
  const response = await api.post<App>(`/v1/apps/${appId}/unpublish`);
  return response.data;
}

// Channel CRUD

export async function addChannel(appId: string, req: AddChannelRequest): Promise<AppChannel> {
  const response = await api.post<AppChannel>(`/v1/apps/${appId}/channels`, req);
  return response.data;
}

export async function updateChannel(
  appId: string,
  channelId: string,
  req: UpdateChannelRequest,
): Promise<AppChannel> {
  const response = await api.patch<AppChannel>(`/v1/apps/${appId}/channels/${channelId}`, req);
  return response.data;
}

export async function deleteChannel(appId: string, channelId: string): Promise<void> {
  await api.delete(`/v1/apps/${appId}/channels/${channelId}`);
}

export async function triggerChannel(
  appId: string,
  channelId: string,
): Promise<{ session_id: string; created_session: boolean }> {
  const response = await api.post<{ session_id: string; created_session: boolean }>(
    `/v1/apps/${appId}/channels/${channelId}/trigger`,
    {},
  );
  return response.data;
}

export async function listAppRuns(
  appId: string,
): Promise<{ data: AppRunEvent[]; buckets?: AppRunBucket[] }> {
  const response = await api.get<{ data: AppRunEvent[]; buckets?: AppRunBucket[] }>(
    `/v1/apps/${appId}/runs?window=24h&groupBy=hour`,
  );
  return response.data;
}

/**
 * Create an A2A (Agent2Agent) channel. The server generates a fresh API key
 * and returns the plaintext exactly once — persist it immediately.
 */
export async function addA2aChannel(
  appId: string,
  req: AddA2aChannelRequest,
): Promise<A2aChannelKeyResponse> {
  const response = await api.post<A2aChannelKeyResponse>(`/v1/apps/${appId}/a2a-channels`, req);
  return response.data;
}

/**
 * Regenerate an A2A channel API key. Returns the new plaintext exactly once;
 * the previous key stops working as soon as this call returns.
 */
export async function regenerateA2aChannelKey(
  appId: string,
  channelId: string,
): Promise<A2aChannelKeyResponse> {
  const response = await api.post<A2aChannelKeyResponse>(
    `/v1/apps/${appId}/a2a-channels/${channelId}/regenerate-key`,
    {},
  );
  return response.data;
}

/** Get the Slack App manifest for an app. Returns manifest YAML and create URL. */
export async function getSlackManifest(
  appId: string,
): Promise<{ manifest_yaml: string; create_url: string } | null> {
  try {
    const response = await api.get<{ manifest_yaml: string; create_url: string }>(
      `/v1/apps/${appId}/slack/manifest`,
    );
    return response.data;
  } catch {
    return null;
  }
}
