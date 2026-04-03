// Agent and Harness types

import type {
  AgentCapabilityConfig,
  InitialFile,
  NetworkAccessList,
  ToolDefinition,
  TokenUsage,
} from "./common-types";

// ============================================
// Agent types (M2)
// ============================================

export type AgentStatus = "active" | "archived" | "deleted";

export interface Agent {
  id: string;
  name: string;
  description: string | null;
  system_prompt: string;
  default_model_id: string | null;
  tags: string[];
  /** Capabilities with per-agent configuration */
  capabilities: AgentCapabilityConfig[];
  initial_files: InitialFile[];
  /** Tool definitions (including client-side tools), defaults to [] */
  tools?: ToolDefinition[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  status: AgentStatus;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
  /** Cumulative token usage across all sessions for this agent */
  usage?: TokenUsage;
}

export interface CreateAgentRequest {
  name: string;
  description?: string;
  system_prompt: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  initial_files?: InitialFile[];
  /** Tool definitions (including client-side tools) */
  tools?: ToolDefinition[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList;
}

export interface UpdateAgentRequest {
  name?: string;
  description?: string;
  system_prompt?: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  initial_files?: InitialFile[];
  status?: AgentStatus;
  /** Tool definitions (including client-side tools) */
  tools?: ToolDefinition[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
}

/** Read-only agent example defined in code, adoptable as a real Agent */
export interface AgentExample {
  slug: string;
  name: string;
  description: string;
  tags: string[];
  capabilities: AgentCapabilityConfig[];
  dev_only: boolean;
}

/** Request to preview the final agent shape with capabilities applied */
export interface PreviewAgentRequest {
  /** The base system prompt (before capability additions) */
  system_prompt: string;
  /** Capabilities to apply with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  /** Client-side tools to include in preview output */
  tools?: ToolDefinition[];
}

/** Response showing the final agent shape after applying capabilities */
export interface AgentPreviewResponse {
  /** The full system prompt with capability additions prepended */
  system_prompt: string;
  /** All tool definitions from capabilities */
  tools: ToolDefinition[];
}

// ============================================
// Harness types
// ============================================

export type HarnessStatus = "active" | "archived" | "deleted";

export interface Harness {
  id: string;
  name: string;
  description: string | null;
  system_prompt: string;
  parent_harness_id: string | null;
  default_model_id: string | null;
  tags: string[];
  /** Capabilities with per-harness configuration */
  capabilities: AgentCapabilityConfig[];
  initial_files: InitialFile[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  /** Whether this harness is built-in (system-managed, readonly) */
  is_built_in: boolean;
  status: HarnessStatus;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
}

export interface CreateHarnessRequest {
  name: string;
  description?: string;
  system_prompt: string;
  parent_harness_id?: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-harness configuration */
  capabilities?: AgentCapabilityConfig[];
  initial_files?: InitialFile[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList;
}

export interface UpdateHarnessRequest {
  name?: string;
  description?: string;
  system_prompt?: string;
  parent_harness_id?: string | null;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-harness configuration */
  capabilities?: AgentCapabilityConfig[];
  initial_files?: InitialFile[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  status?: HarnessStatus;
}

/** Request to preview the final harness shape with capabilities applied */
export interface PreviewHarnessRequest {
  /** The base system prompt (before capability additions) */
  system_prompt: string;
  /** Optional parent harness to inherit from before previewing local changes */
  parent_harness_id?: string;
  /** Capability IDs to apply */
  capabilities?: AgentCapabilityConfig[];
}
