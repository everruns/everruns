// App API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import type { App, CreateAppRequest, UpdateAppRequest, ListResponse } from "./types";

export async function getApps(): Promise<App[]> {
  const response = await api.get<ListResponse<App>>("/v1/apps");
  return response.data.data;
}

export async function getApp(appId: string): Promise<App> {
  const response = await api.get<App>(`/v1/apps/${appId}`);
  return response.data;
}

export async function createApp(data: CreateAppRequest): Promise<App> {
  const response = await api.post<App>("/v1/apps", data);
  return response.data;
}

export async function updateApp(appId: string, data: UpdateAppRequest): Promise<App> {
  const response = await api.patch<App>(`/v1/apps/${appId}`, data);
  return response.data;
}

export async function deleteApp(appId: string): Promise<void> {
  await api.delete(`/v1/apps/${appId}`);
}

export async function publishApp(appId: string): Promise<App> {
  const response = await api.post<App>(`/v1/apps/${appId}/publish`);
  return response.data;
}

export async function unpublishApp(appId: string): Promise<App> {
  const response = await api.post<App>(`/v1/apps/${appId}/unpublish`);
  return response.data;
}
