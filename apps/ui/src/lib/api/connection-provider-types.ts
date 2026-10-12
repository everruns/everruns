import type { McpServerIcon } from "./mcp-server-types";

export interface UserConnection {
  provider: string;
  connection_type: string;
  provider_username?: string;
  scopes?: string;
  connected_at: string;
}

export interface ConnectionProvider {
  provider_id: string;
  display_name: string;
  description: string;
  icon: string;
  /** Theme-neutral icon published by an MCP server. */
  icon_url?: string | null;
  /** Icons published by an MCP server, including light and dark variants. */
  icons?: McpServerIcon[];
  /** Operator slug when `display_name` is a discovered title or a plugin name. */
  slug?: string | null;
  connection_type: "oauth" | "api_key";
  capabilities: string[];
  form_schema?: ConnectionFormSchema;
}

export interface ConnectionFormSchema {
  fields: ConnectionFormField[];
  instructions_markdown: string;
}

export interface ConnectionFormField {
  name: string;
  label: string;
  field_type: "password" | "text" | "url";
  required: boolean;
  placeholder?: string;
  help_text?: string;
}

export interface VerifyConnectionResponse {
  valid: boolean;
  error?: string;
}
