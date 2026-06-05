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

export type PrincipalKind = "user" | "agent_identity" | "system";

export interface PrincipalSummary {
  id: string;
  kind: PrincipalKind;
  subject_id?: string | null;
  metadata: Record<string, unknown>;
}

// ============================================
// Network access list (used by agents, harnesses, sessions)
// ============================================

/**
 * Controls which hosts/URLs an agent session can reach.
 * Merged across layers: allowed=intersect, blocked=union.
 */
export interface NetworkAccessList {
  /** Allowed host patterns (e.g. "example.com", "*.github.com", "https://api.example.com/v1/") */
  allowed?: string[];
  /** Blocked host patterns. Always denied, even if matched by allowed. */
  blocked?: string[];
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
  /**
   * Actual cost of this generation in USD, as reported by the provider inline
   * (e.g. OpenRouter's `usage.cost`). Absent for providers that do not report one.
   */
  actual_cost_usd?: number;
  /**
   * Estimated cost of this generation in USD, derived from the model's price-table
   * profile. Absent when there is no profile cost data for the model. Tracked
   * independently of `actual_cost_usd` so estimate-vs-actual drift can be reconciled.
   */
  estimated_cost_usd?: number;
}

export interface ContextReportSection {
  key: string;
  label: string;
  tokens: number;
  items: number;
}

export interface ContextReportContribution {
  section_key: string;
  source_id: string;
  label: string;
  tokens: number;
}

export interface SessionContextReport {
  session_id: string;
  model: string;
  context_window_tokens?: number;
  estimated_input_tokens: number;
  sections: ContextReportSection[];
  contributions: ContextReportContribution[];
  cumulative_usage?: TokenUsage;
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
