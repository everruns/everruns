// User MCP servers: servers a person adds for themselves.
// See knowledge/integrations/user-mcp-servers.md.

import { api } from "./client";
import type {
  AddUserMcpServerRequest,
  McpServerTool,
  McpToolLabel,
  UpdateUserMcpServerRequest,
  UserMcpServer,
  UserMcpServersResponse,
} from "./types";

const base = (identityId: string) =>
  `/v1/virtual-users/${encodeURIComponent(identityId)}/mcp-servers`;

export async function listUserMcpServers(identityId = "me"): Promise<UserMcpServer[]> {
  const response = await api.get<UserMcpServersResponse>(base(identityId));
  return response.data.data;
}

export async function addUserMcpServer(
  request: AddUserMcpServerRequest,
  identityId = "me",
): Promise<UserMcpServer> {
  const response = await api.post<UserMcpServer>(base(identityId), request);
  return response.data;
}

export async function updateUserMcpServer(
  serverId: string,
  request: UpdateUserMcpServerRequest,
  identityId = "me",
): Promise<UserMcpServer> {
  const response = await api.patch<UserMcpServer>(
    `${base(identityId)}/${encodeURIComponent(serverId)}`,
    request,
  );
  return response.data;
}

export async function removeUserMcpServer(serverId: string, identityId = "me"): Promise<void> {
  await api.delete(`${base(identityId)}/${encodeURIComponent(serverId)}`);
}

/** Tools of one of the person's custom servers, asked live, with their labels. */
export async function getUserMcpServerTools(
  serverId: string,
  identityId = "me",
): Promise<McpServerTool[]> {
  const response = await api.get<{ data: McpServerTool[] }>(
    `${base(identityId)}/${encodeURIComponent(serverId)}/tools`,
  );
  return response.data.data;
}

/** Set the person's label for one tool of their custom server; `null` clears it. */
export async function setUserMcpToolLabel(
  serverId: string,
  toolName: string,
  label: McpToolLabel | null,
  identityId = "me",
): Promise<McpServerTool> {
  const response = await api.put<McpServerTool>(
    `${base(identityId)}/${encodeURIComponent(serverId)}/tools/${encodeURIComponent(toolName)}/label`,
    { label },
  );
  return response.data;
}
