import type { McpServerActsAs } from "./schema-types";

export interface ScopedMcpServer {
  type?: "http";
  url?: string;
  headers?: Record<string, string>;
  use?: string;
  actsAs?: McpServerActsAs;
  tool_discovery?: boolean;
}

export type ScopedMcpServers = Record<string, ScopedMcpServer>;

export interface AgentMcpAttachmentSourceInfo {
  source: "capability" | "harness" | "agent";
  source_label: string;
}

export interface AgentMcpAttachment {
  name: string;
  source: "capability" | "harness" | "agent";
  source_label: string;
  overridden_sources: AgentMcpAttachmentSourceInfo[];
  acts_as: McpServerActsAs;
  preset_name?: string | null;
  preset_id?: string | null;
  connection_provider?: string | null;
  url?: string | null;
  header_names: string[];
  tools_available: boolean;
  tools: string[];
  state: "ready" | "connection_missing" | "preset_missing";
  action: "none" | "connect" | "authorize" | "ask_admin";
  connected_as?: string | null;
  editable: boolean;
}

declare module "./legacy-api-types" {
  interface Agent {
    /** MCP attachments authored directly on the agent. */
    mcpServers?: ScopedMcpServers;
  }

  interface CreateAgentRequest {
    /** MCP attachments authored directly on the agent. */
    mcpServers?: ScopedMcpServers;
  }

  interface UpdateAgentRequest {
    /** Replace MCP attachments authored directly on the agent. */
    mcpServers?: ScopedMcpServers;
  }
}
