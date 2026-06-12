// Capability types

import type { CapabilityId, ToolDefinition } from "./common-types";

// NOTE: CapabilityId is defined in common-types for proper ordering

export type CapabilityStatus = "available" | "coming_soon" | "deprecated";

export interface Capability {
  id: CapabilityId;
  name: string;
  description: string;
  status: CapabilityStatus;
  icon?: string;
  category?: string;
  /** System prompt addition contributed by this capability */
  system_prompt?: string;
  /** Tool definitions provided by this capability */
  tool_definitions?: ToolDefinition[];
  /** Whether this is an MCP server capability */
  is_mcp?: boolean;
  /** IDs of capabilities that this capability depends on */
  dependencies?: CapabilityId[];
  /** UI feature strings this capability contributes to */
  features?: string[];
  /** JSON Schema for capability-specific config */
  config_schema?: Record<string, unknown>;
  /** react-jsonschema-form uiSchema hints for rendering config_schema */
  config_ui_schema?: Record<string, unknown>;
  /** Number of active agents in the org referencing this capability */
  agent_count?: number;
  /** Number of active harnesses in the org referencing this capability */
  harness_count?: number;
  /** Slug under https://dev.everruns.com/capabilities/ for the public docs page */
  docs_slug?: string;
  /**
   * Localized display strings keyed by lowercase language tag (e.g. "uk").
   * The "en" entry carries only config_description, since the base
   * name/description/config_schema strings are already English.
   */
  localizations?: Record<string, CapabilityLocalization>;
}

/** Localized display strings for one locale of a capability. */
export interface CapabilityLocalization {
  name?: string;
  description?: string;
  /** One-line summary of what this capability's config controls */
  config_description?: string;
  /**
   * Overlay merged into config_schema before rendering: mirrors the schema
   * structure (properties/items) with title/description/enum_labels leaves.
   */
  config_overlay?: Record<string, unknown>;
}

export interface DeclarativeCapabilityFile {
  path: string;
  content: string;
  access?: "readonly" | "readwrite";
}

export interface DeclarativeCapabilitySkillFile {
  path: string;
  content: string;
}

export interface DeclarativeCapabilitySkill {
  name: string;
  description: string;
  instructions: string;
  files?: DeclarativeCapabilitySkillFile[];
  user_invocable?: boolean;
  disable_model_invocation?: boolean;
}

export interface DeclarativeCapabilityDefinition {
  name: string;
  display_name?: string | null;
  description: string;
  icon?: string;
  category?: string;
  system_prompt?: string;
  mcp_servers?: Record<string, unknown>;
  skills?: DeclarativeCapabilitySkill[];
  files?: DeclarativeCapabilityFile[];
  dependencies?: CapabilityId[];
  features?: string[];
  risk_level?: "low" | "medium" | "high";
}

export interface DeclarativeCapability {
  id: string;
  capability_id: CapabilityId;
  name: string;
  display_name?: string | null;
  description: string;
  status: "active" | "disabled" | "archived" | "deleted";
  definition: DeclarativeCapabilityDefinition;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export interface CreateDeclarativeCapabilityRequest {
  definition: DeclarativeCapabilityDefinition;
}

export interface UpdateDeclarativeCapabilityRequest {
  definition?: DeclarativeCapabilityDefinition;
  status?: "active" | "disabled" | "archived";
}
