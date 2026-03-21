// App API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import { createCrudApi } from "./crud";
import type { App, CreateAppRequest, UpdateAppRequest } from "./types";

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
