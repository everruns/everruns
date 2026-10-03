import type {
  AgentCapabilityConfig,
  AgentFinding,
  InitialFile,
  ToolDefinition,
} from "./legacy-api-types";
import type { ScopedMcpServers } from "./agent-mcp-types";

/** Request to preview the final agent shape with capabilities applied */
export interface PreviewAgentRequest {
  /** Harness whose effective configuration is layered beneath this draft */
  harness_id?: string;
  initial_files?: InitialFile[];
  mcpServers?: ScopedMcpServers;
  /** The base system prompt (before capability additions) */
  system_prompt: string;
  /** Capabilities to apply with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  /** Client-side tools to include in preview output */
  tools?: ToolDefinition[];
}

/** Response showing the final agent shape after applying capabilities */
export interface AgentPreviewResponse {
  /** Session features from effective capabilities and their dependencies */
  features?: string[];
  initial_files?: InitialFile[];
  /** The full system prompt with capability additions prepended */
  system_prompt: string;
  /** All tool definitions from capabilities */
  tools: ToolDefinition[];
  /** Advisory findings from built-in checks (absent on harness preview) */
  findings?: AgentFinding[];
}
