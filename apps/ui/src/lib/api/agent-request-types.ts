// Agent create/update request bodies, hand-maintained as UI compatibility over the
// generated OpenAPI schemas. Split out of legacy-api-types.ts (size ratchet) and
// re-exported from there; augmentations in agent-*-types.ts target this module.
import type { AgentStatus, ConversationStarter } from "./agent-types";
import type {
  AgentCapabilityConfig,
  InitialFile,
  NetworkAccessList,
  ToolDefinition,
} from "./legacy-api-types";
import type { Communication } from "./schema-types";

export interface CreateAgentRequest {
  service_virtual_user_id?: string;
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
  description?: string;
  /** Markdown intro for fresh Platform Chat threads (agent wins). */
  intro_markdown?: string | null;
  /** One-line description in simplified Markdown (agent wins). */
  short_description?: string | null;
  /** Conversation starters (agent wins when non-empty). */
  starters?: ConversationStarter[];
  system_prompt: string;
  /** Base execution harness (id). Mutually exclusive with `harness_name`. Omit both to use the organization default (Conversation for new organizations). */
  harness_id?: string;
  /** Base execution harness (name), resolved within the org. Mutually exclusive with `harness_id`. */
  harness_name?: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  /** How the agent talks to people; omit for the default (`direct`) or to leave unchanged. */
  communication?: Communication;
  initial_files?: InitialFile[];
  /** Tool definitions (including client-side tools) */
  tools?: ToolDefinition[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList;
}

export interface UpdateAgentRequest {
  service_virtual_user_id?: string | null;
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name?: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
  description?: string;
  /** Markdown intro; omit to leave unchanged, null clears. */
  intro_markdown?: string | null;
  /** One-line description; omit to leave unchanged, null clears. */
  short_description?: string | null;
  /** Conversation starters; omit to leave unchanged, empty clears. */
  starters?: ConversationStarter[] | null;
  system_prompt?: string;
  /** Base execution harness (id). Omit to leave unchanged; never clearable to null. Mutually exclusive with `harness_name`. */
  harness_id?: string;
  /** Base execution harness (name), resolved within the org. Mutually exclusive with `harness_id`. */
  harness_name?: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  /** How the agent talks to people; omit for the default (`direct`) or to leave unchanged. */
  communication?: Communication;
  initial_files?: InitialFile[];
  status?: AgentStatus;
  /** Tool definitions (including client-side tools) */
  tools?: ToolDefinition[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  /**
   * Session this one was forked from (knowledge/runtime-resources/forking-sessions.md).
   * NULL for sessions that were not forked. Distinct from a subagent's parent.
   */
  forked_from_session_id?: string | null;
  /** Parent event sequence the fork was taken at. NULL unless this is a fork. */
  forked_from_sequence?: number | null;
}
