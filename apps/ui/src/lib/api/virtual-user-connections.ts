// Virtual User Connections API functions
// Identity-scoped (not user-scoped) — connections belong to an virtual user

import { api } from "./client";
import type { UserConnection, VerifyConnectionResponse } from "./types";

export async function listIdentityConnections(identityId: string): Promise<UserConnection[]> {
  const response = await api.get<UserConnection[]>(`/v1/virtual-users/${identityId}/connections`);
  return response.data;
}

export async function createIdentityApiKeyConnection(
  identityId: string,
  provider: string,
  apiKey: string,
  extraFields?: Record<string, string>,
): Promise<UserConnection> {
  const response = await api.post<UserConnection>(
    `/v1/virtual-users/${identityId}/connections/${provider}`,
    { api_key: apiKey, ...extraFields },
  );
  return response.data;
}

export async function deleteIdentityConnection(
  identityId: string,
  provider: string,
): Promise<void> {
  await api.delete(`/v1/virtual-users/${identityId}/connections/${provider}`);
}

export async function verifyIdentityConnection(
  identityId: string,
  provider: string,
): Promise<VerifyConnectionResponse> {
  const response = await api.post<VerifyConnectionResponse>(
    `/v1/virtual-users/${identityId}/connections/${provider}/verify`,
  );
  return response.data;
}
