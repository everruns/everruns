// MCP Server types

/** MCP Server transport type */
export type McpServerTransportType = "http";

/** MCP Server auth mode */
export type McpServerAuthMode = "none" | "api_key" | "oauth";
/**
 * MCP protocol-era adoption policy.
 * - `auto`: probe and adapt (legacy/current/RC) — the default.
 * - `legacy`: pin 2025-03-26 (stateful handshake + session id).
 * - `stable`: pin 2025-06-18 (stateful handshake + session id).
 * - `rc`: pin 2026-07-28 stateless (no handshake).
 */
export type McpProtocolMode = "auto" | "legacy" | "stable" | "rc";
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
  auth_mode: McpServerAuthMode;
  /** Protocol-era policy. Omitted by the API when `auto` (the default). */
  protocol_mode?: McpProtocolMode;
  oauth_provider_id?: string;
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
  auth_mode?: McpServerAuthMode;
  protocol_mode?: McpProtocolMode;
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
  auth_mode?: McpServerAuthMode;
  protocol_mode?: McpProtocolMode;
  api_key?: string;
  headers?: Record<string, string>;
}
