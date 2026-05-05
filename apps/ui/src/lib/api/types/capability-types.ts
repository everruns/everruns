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
}
