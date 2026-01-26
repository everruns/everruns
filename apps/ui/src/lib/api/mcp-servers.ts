// MCP Server API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import type {
  McpServer,
  CreateMcpServerRequest,
  UpdateMcpServerRequest,
  ListResponse,
} from "./types";

// MCP Server CRUD

export async function getMcpServers(): Promise<McpServer[]> {
  const response = await api.get<ListResponse<McpServer>>("/v1/mcp-servers");
  return response.data.data;
}

export async function getMcpServer(serverId: string): Promise<McpServer> {
  const response = await api.get<McpServer>(`/v1/mcp-servers/${serverId}`);
  return response.data;
}

export async function createMcpServer(
  data: CreateMcpServerRequest
): Promise<McpServer> {
  const response = await api.post<McpServer>("/v1/mcp-servers", data);
  return response.data;
}

export async function updateMcpServer(
  serverId: string,
  data: UpdateMcpServerRequest
): Promise<McpServer> {
  const response = await api.patch<McpServer>(`/v1/mcp-servers/${serverId}`, data);
  return response.data;
}

export async function deleteMcpServer(serverId: string): Promise<void> {
  await api.delete(`/v1/mcp-servers/${serverId}`);
}
