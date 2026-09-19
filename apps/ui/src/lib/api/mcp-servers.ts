// MCP Server API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { createCrudApi } from "./crud";
import { api } from "./client";
import type {
  CreateMcpServerRequest,
  McpServer,
  McpServerCatalogEntry,
  McpServerCatalogResponse,
  McpServerUsage,
  UpdateMcpServerRequest,
} from "./types";

export const mcpServersCrudApi = createCrudApi<
  McpServer,
  CreateMcpServerRequest,
  UpdateMcpServerRequest
>("/v1/mcp-servers");

export const getMcpServers = mcpServersCrudApi.list;
export const getMcpServer = mcpServersCrudApi.get;
export const createMcpServer = mcpServersCrudApi.create;
export const updateMcpServer = mcpServersCrudApi.update;
export const deleteMcpServer = mcpServersCrudApi.delete;
export const destroyMcpServer = mcpServersCrudApi.destroy;

export async function getMcpServerCatalog(
  cursor?: string,
): Promise<{ data: McpServerCatalogEntry[]; next_cursor?: string | null }> {
  const params = new URLSearchParams({ limit: "50" });
  if (cursor) params.set("cursor", cursor);
  const response = await api.get<McpServerCatalogResponse>(
    `/v1/mcp-servers/catalog?${params.toString()}`,
  );
  return {
    data: response.data.data as McpServerCatalogEntry[],
    next_cursor: response.data.next_cursor,
  };
}

export async function getMcpServerUsage(serverId: string): Promise<McpServerUsage> {
  const response = await api.get<McpServerUsage>(`/v1/mcp-servers/${serverId}/usage`);
  return response.data;
}
