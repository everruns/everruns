import type { McpElicitationPolicy } from "./schema-types";

// From legacy mcp-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// MCP Server types
/** MCP Server transport type */
export type McpServerTransportType = "http";

/** MCP Server auth mode */
export type McpServerAuthMode = "none" | "api_key" | "oauth";

/** MCP Server status */
export type McpServerStatus = "active" | "disabled" | "archived" | "deleted";

/**
 * MCP protocol-era adoption policy. Pinned values are the spec version dates.
 * - `auto`: probe and adapt across every era — the default.
 * - `2025-03-26`: stateful handshake + session id.
 * - `2025-06-18`: stateful handshake + session id.
 * - `2026-07-28`: stateless (no handshake).
 */
export type McpProtocolMode = "auto" | "2025-03-26" | "2025-06-18" | "2026-07-28";

/** Outcome of an OAuth preset's connection check (discovery + client registration). */
export type McpConnectionCheckStatus =
  | "ready"
  | "blocked_by_network_policy"
  | "unreachable"
  | "failed"
  | "not_checked";

/** Whether an OAuth preset's sign-in service works from this deployment. */
export interface McpConnectionCheck {
  status: McpConnectionCheckStatus;
  checked_at?: string;
  /** Host of the request that failed. */
  host?: string;
  /** Short, safe explanation of a failed check. */
  reason?: string;
}

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
  /** Elicitation policy. Omitted by the API when `url` (the default). */
  elicitation_policy?: McpElicitationPolicy;
  oauth_provider_id?: string;
  /** Agent connection that supplies the service credential (`github`). */
  service_connection_provider?: string;
  /** OAuth presets only: last connection check. */
  connection_check?: McpConnectionCheck;
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
  elicitation_policy?: McpElicitationPolicy;
  /** Agent connection that supplies the service credential; `""` clears it on update. */
  service_connection_provider?: string;
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
  elicitation_policy?: McpElicitationPolicy;
  /** Agent connection that supplies the service credential; `""` clears it on update. */
  service_connection_provider?: string;
  api_key?: string;
  headers?: Record<string, string>;
}
