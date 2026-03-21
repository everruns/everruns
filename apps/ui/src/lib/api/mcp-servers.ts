// MCP Server API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { createCrudApi } from "./crud";
import type { CreateMcpServerRequest, McpServer, UpdateMcpServerRequest } from "./types";

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
