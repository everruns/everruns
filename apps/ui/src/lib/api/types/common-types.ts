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

/**
 * Semantic hints describing a tool's behavioral properties.
 * All fields are optional — undefined means "unspecified".
 * Follows MCP tool annotations convention plus everruns-specific hints.
 */
export interface ToolHints {
  /** Tool does not modify any state (read-only queries, lookups) */
  readonly?: boolean;
  /** Tool may irreversibly destroy or delete data */
  destructive?: boolean;
  /** Same args produce same effect; safe to retry */
  idempotent?: boolean;
  /** Interacts with external entities (network, APIs, cloud services) */
  open_world?: boolean;
  /** Needs API keys or credentials to function */
  requires_secrets?: boolean;
  /** May take significant time to complete (> ~5s typical) */
  long_running?: boolean;
}

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
  /** Semantic hints describing the tool's behavioral properties */
  hints?: ToolHints;
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
  /** Semantic hints describing the tool's behavioral properties */
  hints?: ToolHints;
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
