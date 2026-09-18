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
