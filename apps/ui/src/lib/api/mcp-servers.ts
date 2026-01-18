// MCP Server API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import type {
  McpServer,
  McpOAuthStatus,
  McpOAuthAuthorizationResponse,
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

export async function createMcpServer(data: CreateMcpServerRequest): Promise<McpServer> {
  const response = await api.post<McpServer>("/v1/mcp-servers", data);
  return response.data;
}

export async function updateMcpServer(
  serverId: string,
  data: UpdateMcpServerRequest,
): Promise<McpServer> {
  const response = await api.patch<McpServer>(`/v1/mcp-servers/${serverId}`, data);
  return response.data;
}

export async function deleteMcpServer(serverId: string): Promise<void> {
  await api.delete(`/v1/mcp-servers/${serverId}`);
}

// MCP Server OAuth functions

/**
 * Get OAuth status for an MCP server
 */
export async function getMcpServerOAuthStatus(
  org: string,
  serverId: string
): Promise<McpOAuthStatus> {
  const response = await api.get<McpOAuthStatus>(
    `/v1/orgs/${org}/mcp-servers/${serverId}/oauth/status`
  );
  return response.data;
}

/**
 * Start OAuth authorization flow for an MCP server
 * Returns URL to redirect user to for authorization
 */
export async function startMcpServerOAuth(
  org: string,
  serverId: string,
  returnUrl?: string
): Promise<McpOAuthAuthorizationResponse> {
  const response = await api.post<McpOAuthAuthorizationResponse>(
    `/v1/orgs/${org}/mcp-servers/${serverId}/oauth/authorize`,
    { return_url: returnUrl }
  );
  return response.data;
}

/**
 * Revoke OAuth token for an MCP server
 */
export async function revokeMcpServerOAuth(
  org: string,
  serverId: string
): Promise<void> {
  await api.delete(`/v1/orgs/${org}/mcp-servers/${serverId}/oauth/token`);
}
