// Common/shared types used across multiple domains

// ============================================
// Capability ID and config (used by agents, harnesses, capabilities)
// ============================================

/** Capability ID - extensible string-based identifier */
export type CapabilityId = string;

/** Per-agent capability configuration */
export interface AgentCapabilityConfig {
  /** Reference to the capability ID */
  ref: CapabilityId;
  /** Per-agent configuration for this capability (capability-specific) */
  config: Record<string, unknown>;
}

export interface InitialFile {
  path: string;
  content: string;
  encoding: "text" | "base64";
  is_readonly: boolean;
}

// ============================================
// Tool types
// ============================================

export type ToolPolicy = "auto" | "requires_approval";

/** Tool definition - builtin tool configuration */
export interface BuiltinTool {
  type: "builtin";
  /** Tool name (used by LLM and for registry lookup) */
  name: string;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  /** Tool description for LLM */
  description: string;
  /** JSON schema for tool parameters */
  parameters: Record<string, unknown>;
  /** Tool policy (auto or requires_approval) */
  policy?: ToolPolicy;
}

/** Tool definition - client-side tool executed by the caller */
export interface ClientSideTool {
  type: "client_side";
  /** Tool name (used by LLM and for registry lookup) */
  name: string;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  /** Tool description for LLM */
  description: string;
  /** JSON schema for tool parameters */
  parameters: Record<string, unknown>;
}

/** Tool definition - builtin or client-side */
export type ToolDefinition = BuiltinTool | ClientSideTool;

// ============================================
// Token usage
// ============================================

/** Token usage statistics */
export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  /** Tokens read from cache (reduces cost) */
  cache_read_tokens?: number;
  /** Tokens written to cache (Anthropic-specific) */
  cache_creation_tokens?: number;
}

// ============================================
// List response wrappers
// ============================================

export interface ListResponse<T> {
  data: T[];
}

export interface PaginatedResponse<T> {
  data: T[];
  total: number;
  offset: number;
  limit: number;
}

export interface PaginationParams {
  offset?: number;
  limit?: number;
}

// ============================================
// Health check
// ============================================

export interface HealthResponse {
  status: string;
  version: string;
  runner_mode: string;
}
