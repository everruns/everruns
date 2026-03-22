// Agent Identity Connections API functions
// Identity-scoped (not user-scoped) — connections belong to an agent identity

import { api } from "./client";
import type { UserConnection, VerifyConnectionResponse } from "./types";

export async function listIdentityConnections(identityId: string): Promise<UserConnection[]> {
  const response = await api.get<UserConnection[]>(
    `/v1/agent-identities/${identityId}/connections`,
  );
  return response.data;
}

export async function createIdentityApiKeyConnection(
  identityId: string,
  provider: string,
  apiKey: string,
): Promise<UserConnection> {
  const response = await api.post<UserConnection>(
    `/v1/agent-identities/${identityId}/connections/${provider}`,
    { api_key: apiKey },
  );
  return response.data;
}

export async function deleteIdentityConnection(
  identityId: string,
  provider: string,
): Promise<void> {
  await api.delete(`/v1/agent-identities/${identityId}/connections/${provider}`);
}

export async function verifyIdentityConnection(
  identityId: string,
  provider: string,
): Promise<VerifyConnectionResponse> {
  const response = await api.post<VerifyConnectionResponse>(
    `/v1/agent-identities/${identityId}/connections/${provider}/verify`,
  );
  return response.data;
}
