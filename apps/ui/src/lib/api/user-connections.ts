// User Connections API functions
// User-scoped (not org-scoped) — connections represent user's identity

import { api } from "./client";
import type { UserConnection, ConnectionProvider, VerifyConnectionResponse } from "./types";

export async function getUserConnections(): Promise<UserConnection[]> {
  const response = await api.get<UserConnection[]>("/v1/user/connections");
  return response.data;
}

export async function getConnectionProviders(): Promise<ConnectionProvider[]> {
  const response = await api.get<ConnectionProvider[]>("/v1/user/connections/providers");
  return response.data;
}

export async function createApiKeyConnection(
  provider: string,
  apiKey: string,
  extraFields?: Record<string, string>,
): Promise<UserConnection> {
  const response = await api.post<UserConnection>(`/v1/user/connections/${provider}`, {
    api_key: apiKey,
    ...extraFields,
  });
  return response.data;
}

export async function deleteUserConnection(provider: string): Promise<void> {
  await api.delete(`/v1/user/connections/${provider}`);
}

export async function verifyConnection(provider: string): Promise<VerifyConnectionResponse> {
  const response = await api.post<VerifyConnectionResponse>(
    `/v1/user/connections/${provider}/verify`,
  );
  return response.data;
}
