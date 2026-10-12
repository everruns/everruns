// External AI clients (Claude, ChatGPT, Cursor, ...) the signed-in person
// approved to act as them on /mcp. User-scoped: not tied to the selected org.

import { api } from "./client";
import type { ConnectedClientsResponse } from "./types";

export async function getConnectedClients(): Promise<ConnectedClientsResponse> {
  const response = await api.get<ConnectedClientsResponse>("/v1/user/connected-clients");
  return response.data;
}

export async function revokeConnectedClient(grantId: string): Promise<void> {
  await api.delete(`/v1/user/connected-clients/${encodeURIComponent(grantId)}`);
}
