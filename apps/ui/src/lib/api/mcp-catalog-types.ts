import type { McpServer } from "./legacy-api-types";

export interface McpServerCatalogEntry extends McpServer {
  used_by_agents: number;
}
