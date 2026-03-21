// MCP Server types

/** MCP Server transport type */
export type McpServerTransportType = "http";

/** MCP Server status */
export type McpServerStatus = "active" | "disabled" | "archived" | "deleted";

/** MCP Server configuration */
export interface McpServer {
  id: string;
  name: string;
  description: string | null;
  url: string;
  transport_type: McpServerTransportType;
  status: McpServerStatus;
  api_key_set: boolean;
  headers: Record<string, string>;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
}

/** Request to create an MCP server */
export interface CreateMcpServerRequest {
  name: string;
  description?: string;
  url: string;
  transport_type?: McpServerTransportType;
  api_key?: string;
  headers?: Record<string, string>;
}

/** Request to update an MCP server */
export interface UpdateMcpServerRequest {
  name?: string;
  description?: string;
  url?: string;
  transport_type?: McpServerTransportType;
  status?: McpServerStatus;
  api_key?: string;
  headers?: Record<string, string>;
}
