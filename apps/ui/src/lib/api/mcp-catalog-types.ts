import type { McpServer } from "./mcp-server-types";

export interface McpServerCatalogEntry extends McpServer {
  used_by_agents: number;
}
