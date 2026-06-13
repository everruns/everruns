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
  /** Addressable name (slug): lowercase alphanumeric and hyphens (e.g. "customer-support") */
  name: string;
  /** Human-readable display name shown in UI. Falls back to name when absent. */
  display_name: string | null;
  description: string | null;
  system_prompt: string;
  default_model_id: string | null;
  default_version_id?: string | null;
  forked_from_agent_id?: string | null;
  forked_from_version_id?: string | null;
  root_agent_id?: string | null;
  tags: string[];
  /** Capabilities with per-agent configuration */
  capabilities: AgentCapabilityConfig[];
  /** Initial files. Optional: older records and serializers that strip empty arrays may omit this field. */
  initial_files?: InitialFile[];
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
  /** Number of sessions using this agent. Present on list/detail API responses. */
  session_count?: number;
  /** Number of non-deleted apps using this agent. Present on list/detail API responses. */
  app_count?: number;
}

export type AgentVersionChangeKind =
  | "auto"
  | "manual"
  | "patch"
  | "minor"
  | "major"
  | "import"
  | "rollback"
  | "fork";

export interface AgentVersion {
  id: string;
  agent_id: string;
  version_number: number;
  semver_major: number;
  semver_minor: number;
  semver_patch: number;
  version: string;
  is_published: boolean;
  parent_version_id: string | null;
  source_version_id: string | null;
  created_by_principal_id: string | null;
  change_kind: AgentVersionChangeKind;
  summary: string | null;
  config_hash: string;
  authored_config: Record<string, unknown>;
  resolved_config: Record<string, unknown>;
  created_at: string;
}

export interface CreateAgentVersionRequest {
  summary?: string;
  change_kind?: AgentVersionChangeKind;
}

export interface SetDefaultAgentVersionRequest {
  version_id: string;
}

export interface RollbackAgentVersionRequest {
  save_version: boolean;
  summary?: string;
}

export interface ForkAgentVersionRequest {
  name: string;
  display_name?: string;
  description?: string;
}

export interface AgentVersionDiffResponse {
  from_version_id: string;
  to_version_id: string;
  authored_diff: Record<string, { from: unknown; to: unknown }>;
  resolved_diff: Record<string, { from: unknown; to: unknown }>;
}

export interface CreateAgentRequest {
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
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
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name?: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
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
  name: string;
  display_name: string;
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

export type FindingSeverity = "warning" | "info" | "suggestion";

export type FindingCategory = "structure" | "completeness" | "effectiveness" | "safety" | "cost";

export type FindingSource = "builtin" | "llm" | "health_check";

/** Pointer to the config field (and optional byte span) a finding refers to */
export interface FindingLocation {
  field: string;
  start?: number;
  end?: number;
}

/** Advisory finding from agent config checks (specs/agent-checks.md) */
export interface AgentFinding {
  /** Stable rule identifier, e.g. "prompt.duplicate_paragraphs" */
  rule_id: string;
  severity: FindingSeverity;
  category: FindingCategory;
  message: string;
  location?: FindingLocation;
  /** Proposed replacement text (phase 2+) */
  fix?: string;
  source: FindingSource;
}

/** Response from on-demand agent analysis (built-in rules + LLM checkers) */
export interface AgentAnalysisResponse {
  findings: AgentFinding[];
}

export type HealthCheckStatus = "pending" | "running" | "completed" | "failed";

/** Outcome of a single generated health-check case */
export interface HealthCheckCaseResult {
  name: string;
  user_message: string;
  rubric: string;
  /** Public ID of the real session created for this case */
  session_id?: string;
  passed: boolean;
  score: number;
  judge_reason: string;
  deterministic_reason: string;
  turns: number;
  latency_ms: number;
  error?: string;
}

/** Aggregate metrics across all cases in a health-check run */
export interface HealthCheckSummary {
  total: number;
  passed: number;
  failed: number;
  errored: number;
  pass_rate: number;
  avg_score: number;
  avg_turns: number;
  total_input_tokens: number;
  total_output_tokens: number;
}

/** A behavioral health-check run (specs/agent-checks.md, tier-3) */
export interface HealthCheckRun {
  id: string;
  agent_id?: string;
  config_hash: string;
  model_id?: string;
  status: HealthCheckStatus;
  summary?: HealthCheckSummary;
  results?: HealthCheckCaseResult[];
  error_message?: string;
  created_at: string;
  completed_at?: string;
}

/** Response showing the final agent shape after applying capabilities */
export interface AgentPreviewResponse {
  /** The full system prompt with capability additions prepended */
  system_prompt: string;
  /** All tool definitions from capabilities */
  tools: ToolDefinition[];
  /** Advisory findings from built-in checks (absent on harness preview) */
  findings?: AgentFinding[];
}

// ============================================
// Harness types
// ============================================

export type HarnessStatus = "active" | "archived" | "deleted";

export interface Harness {
  id: string;
  /** Addressable name (slug): lowercase alphanumeric and hyphens (e.g. "my-harness") */
  name: string;
  /** Human-readable display name shown in UI. Falls back to name when absent. */
  display_name: string | null;
  description: string | null;
  system_prompt: string;
  parent_harness_id: string | null;
  default_model_id: string | null;
  tags: string[];
  /** Capabilities with per-harness configuration */
  capabilities: AgentCapabilityConfig[];
  /** Initial files. Optional: older records and serializers that strip empty arrays may omit this field. */
  initial_files?: InitialFile[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  /** Whether this harness is built-in (system-managed, readonly) */
  is_built_in: boolean;
  status: HarnessStatus;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
  /** Number of sessions using this harness. Present on list/detail API responses. */
  session_count?: number;
  /** Number of non-deleted apps using this harness. Present on list/detail API responses. */
  app_count?: number;
}

export interface CreateHarnessRequest {
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
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
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name?: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
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

/** Read-only harness example defined in code, adoptable as a real Harness */
export interface HarnessExample {
  name: string;
  display_name: string;
  description: string;
  tags: string[];
  /** Name of the parent harness (e.g. `generic`) the example will inherit from when imported. */
  parent_name?: string;
  capabilities: AgentCapabilityConfig[];
  dev_only: boolean;
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
