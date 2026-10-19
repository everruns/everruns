// MCP servers added to one chat only (user_mcp add with scope "chat").
// See knowledge/integrations/user-mcp-servers.md (D6).

import { api } from "./client";
import type { ChatMcpServer } from "./types";

const base = (sessionId: string) => `/v1/sessions/${encodeURIComponent(sessionId)}/mcp-servers`;

export async function listChatMcpServers(sessionId: string): Promise<ChatMcpServer[]> {
  const response = await api.get<ChatMcpServer[]>(base(sessionId));
  return response.data;
}

export async function removeChatMcpServer(sessionId: string, name: string): Promise<void> {
  await api.delete(`${base(sessionId)}/${encodeURIComponent(name)}`);
}
