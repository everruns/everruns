// MCP Server API functions
// All routes are org-scoped: /v1/orgs/{org}/mcp-servers/...

import { api } from "./client";
import type {
  McpServer,
  CreateMcpServerRequest,
  UpdateMcpServerRequest,
  ListResponse,
} from "./types";

// MCP Server CRUD

export async function getMcpServers(org: string): Promise<McpServer[]> {
  const response = await api.get<ListResponse<McpServer>>(`/v1/orgs/${org}/mcp-servers`);
  return response.data.data;
}

export async function getMcpServer(org: string, serverId: string): Promise<McpServer> {
  const response = await api.get<McpServer>(`/v1/orgs/${org}/mcp-servers/${serverId}`);
  return response.data;
}

export async function createMcpServer(
  org: string,
  data: CreateMcpServerRequest
): Promise<McpServer> {
  const response = await api.post<McpServer>(`/v1/orgs/${org}/mcp-servers`, data);
  return response.data;
}

export async function updateMcpServer(
  org: string,
  serverId: string,
  data: UpdateMcpServerRequest
): Promise<McpServer> {
  const response = await api.patch<McpServer>(`/v1/orgs/${org}/mcp-servers/${serverId}`, data);
  return response.data;
}

export async function deleteMcpServer(org: string, serverId: string): Promise<void> {
  await api.delete(`/v1/orgs/${org}/mcp-servers/${serverId}`);
}
