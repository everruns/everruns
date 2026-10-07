import type { McpConnectInChat, McpServerActsAs } from "./schema-types";

export interface ScopedMcpServer {
  type?: "http";
  url?: string;
  headers?: Record<string, string>;
  use?: string;
  actsAs?: McpServerActsAs;
  /** `never` fails a call missing a sign-in with a settings link instead of a card. */
  connectInChat?: McpConnectInChat;
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
