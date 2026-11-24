import { api } from "./client";
import type { App, ListResponse } from "./types";

export async function getApps(includeArchived = false): Promise<App[]> {
  const query = includeArchived ? "?include_archived=true" : "";
  const response = await api.get<ListResponse<App>>(`/v1/platform/apps${query}`);
  return response.data.data;
}

export async function getApp(appId: string): Promise<App> {
  const response = await api.get<App>(`/v1/platform/apps/${appId}`);
  return response.data;
}
