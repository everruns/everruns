import type {
  AgentCapabilityConfig,
  InitialFile,
  NetworkAccessList,
  TokenUsage,
  ToolDefinition,
} from "./legacy-api-types";
import type { OpenApiAgentChannelSummary } from "./schema-types";

export type AgentStatus = "active" | "archived" | "deleted";

export interface AgentHarnessSummary {
  id: string | null;
  name: string | null;
  display_name: string | null;
  source: "explicit" | "organization_default";
  status: "active" | "archived" | "deleted" | "unresolved";
}

export type AgentChannelSummary = OpenApiAgentChannelSummary;

/**
 * A conversation starter shown on a fresh Platform Chat thread. Selecting one
 * inserts its text into the composer. `icon` reuses the harness icon name set
 * (`HarnessIcon`); unknown names fall back to the default glyph.
 */
export interface ConversationStarter {
  /** Optional icon name from the harness icon set (e.g. "zap"). */
  icon?: string | null;
  /** Prompt text inserted into the composer when selected. */
  text: string;
}

export interface Agent {
  service_virtual_user_id?: string | null;
  id: string;
  /** Addressable name (slug): lowercase alphanumeric and hyphens (e.g. "customer-support") */
  name: string;
  /** Human-readable display name shown in UI. Falls back to name when absent. */
  display_name: string | null;
  description: string | null;
  /**
   * Optional Markdown intro rendered as an intro box at the top of a fresh
   * Platform Chat thread. Images are allowed. Wins over the harness intro.
   * Hidden once the user inputs.
   */
  intro_markdown?: string | null;
  /**
   * Optional one-line description in simplified Markdown, shown below the chat
   * title once the intro is hidden. Wins over the harness value.
   */
  short_description?: string | null;
  /**
   * Conversation starters for a fresh Platform Chat thread. Win over the
   * harness starters when non-empty.
   */
  starters?: ConversationStarter[];
  system_prompt: string;
  /** Base execution harness this agent runs on. Required; defaults to the organization default (Conversation for new organizations). */
  harness_id: string;
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
  /**
   * The agent-level incident switch (EVE-1007). When true every channel of
   * this agent refuses traffic, whatever its own publish state, and resuming
   * restores each one to where it was.
   */
  exposures_suspended?: boolean;
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
  /** Harness a newly created session for this agent will resolve to. */
  effective_harness?: AgentHarnessSummary;
  /** Non-secret inbound channel summaries; schedules are separate triggers. */
  channels?: AgentChannelSummary[];
}
