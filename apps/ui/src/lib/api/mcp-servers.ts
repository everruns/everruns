// MCP Server API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { createCrudApi } from "./crud";
import { api } from "./client";
import type {
  CreateMcpServerRequest,
  McpServer,
  McpServerCatalogEntry,
  McpServerCatalogResponse,
  McpServerTool,
  McpServerUsage,
  McpToolLabel,
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

/** The tools a server offers, with each tool's saved label and any suggestion. */
export async function getMcpServerTools(serverId: string): Promise<McpServerTool[]> {
  const response = await api.get<McpServerTool[]>(`/v1/mcp-servers/${serverId}/tools`);
  return response.data;
}

/** Set a person's label for one tool; `null` goes back to the default. */
export async function setMcpToolLabel(
  serverId: string,
  toolName: string,
  label: McpToolLabel | null,
): Promise<McpServerTool> {
  const response = await api.put<McpServerTool>(
    `/v1/mcp-servers/${serverId}/tools/${encodeURIComponent(toolName)}/label`,
    { label },
  );
  return response.data;
}

/** Ask for suggested labels for every unlabeled tool; returns the refreshed list. */
export async function suggestMcpToolLabels(serverId: string): Promise<McpServerTool[]> {
  const response = await api.post<McpServerTool[]>(
    `/v1/mcp-servers/${serverId}/tools/suggest-labels`,
    {},
  );
  return response.data;
}
