// User Connections API functions
// User-scoped (not org-scoped) — connections represent user's identity

import { api } from "./client";
import type { UserConnection } from "./types";

export async function getUserConnections(): Promise<UserConnection[]> {
  const response = await api.get<UserConnection[]>("/v1/user/connections");
  return response.data;
}

export async function deleteUserConnection(provider: string): Promise<void> {
  await api.delete(`/v1/user/connections/${provider}`);
}

export async function putApiKeyConnection(provider: string, apiKey: string): Promise<void> {
  await api.put(`/v1/user/connections/api-key/${provider}`, { api_key: apiKey });
}
