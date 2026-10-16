import type {
  InitialFile,
  NetworkAccessList,
  PrincipalSummary,
  TokenUsage,
  ToolDefinition,
} from "./legacy-api-types";
import type { SessionActivity, SessionSource } from "./schema-types";

// From legacy session-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// ============================================
// Session types (M2)
// ============================================
// Session status values:
// - "started": Session just created, no turn executed yet
// - "active": A turn is currently running
// - "idle": Turn completed, session waiting for next input
// - "waiting_for_tool_results": Session paused, waiting for client-side tool results
export type SessionStatus = "started" | "active" | "idle" | "waiting_for_tool_results" | "paused";

export interface Session {
  id: string;
  /** Organization this session belongs to */
  organization_id: string;
  /**
   * Workspace this session is attached to (wsp_<32-hex>); owns the virtual
   * filesystem. For the default 1:1 case this equals the session id. Optional
   * here until the UI consumes it directly instead of deriving from the id.
   */
  workspace_id?: string;
  harness_id: string;
  agent_id: string | null;
  /** Revision of the agent's history this session started on. */
  agent_revision?: number | null;
  virtual_user_id?: string | null;
  /** Fixed end-user subject for an organisation-shared Playground conversation. */
  playground_user_id?: string | null;
  owner_principal_id: string;
  resolved_owner_user_id?: string | null;
  owner?: PrincipalSummary | null;
  effective_owner?: PrincipalSummary | null;
  title: string | null;
  /** Objective shown in session lists and injected into the agent prompt */
  goal?: string | null;
  locale?: string | null;
  /** Preview text from the first user message (truncated) */
  preview?: string | null;
  /** Preview text from the last assistant response (truncated) */
  output_preview?: string | null;
  tags: string[];
  model_id: string | null;
  /** Tool definitions (including client-side tools), defaults to [] */
  tools?: ToolDefinition[];
  status: SessionStatus;
  /**
   * How the session was started (EVE-852). Server-owned for every ingress
   * except `chat`/`api`, which is what makes the Sessions source facet
   * trustworthy. Optional here so responses from an older server still parse.
   */
  source?: SessionSource;
  /**
   * Outcome-oriented view of the session, derived server-side from its status
   * and the outcome of its most recent turn. `status` alone has no notion of
   * failure, which is why the list and the facet rail read this instead.
   */
  activity?: SessionActivity;
  last_turn_status?: string | null;
  /**
   * Generated one-sentence description of what the run did, and where it
   * failed (EVE-867). Absent for chat threads, for runs with no terminal turn
   * yet, and on deployments with no utility LLM — readers must have a fallback.
   */
  run_summary?: string;
  /** Latest user or completed assistant message; list projection, independent of session edits. */
  last_message_at?: string | null;
  created_at: string;
  updated_at: string;
  started_at: string | null;
  finished_at: string | null;
  /** Cumulative token usage for all LLM calls in this session */
  usage?: TokenUsage;
  /** Whether this session is pinned by the current user */
  is_pinned?: boolean;
  /**
   * When this session was archived; absent means active. Archived sessions are
   * hidden from default list results and shown by passing
   * `include_archived=true`. Unlike `is_pinned`, archive is a property of the
   * session itself rather than of the viewer.
   */
  archived_at?: string | null;
  /** Number of active schedules for this session */
  active_schedule_count?: number;
  /**
   * Total events recorded for this session. Denormalized counter, omitted when
   * zero — an empty tab carries no badge (EVE-868).
   */
  event_count?: number;
  /**
   * Background work the session owns — subagents, external agents, background
   * tools. This is what the Work tab holds; `active_schedule_count` describes
   * only the schedules it also lists. Omitted when zero.
   */
  task_count?: number;
  /**
   * Non-directory files in this session's workspace. Persisted files only:
   * capability-provided virtual mounts are served from memory and excluded.
   * Omitted when zero.
   */
  file_count?: number;
  /** Aggregated UI features from all active capabilities (harness + agent + session) */
  features?: string[];
  /** Session-level system prompt override (prepended to agent prompt) */
  system_prompt?: string | null;
  /** Session-level initial files (additive to agent initial_files) */
  initial_files?: InitialFile[];
  /** Session-level client hints (defaults for every turn) */
  hints?: Record<string, unknown>;
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  /**
   * Session this one was forked from (knowledge/runtime-resources/forking-sessions.md).
   * NULL for sessions that were not forked. Distinct from a subagent's parent.
   */
  forked_from_session_id?: string | null;
  /** Parent event sequence the fork was taken at. NULL unless this is a fork. */
  forked_from_sequence?: number | null;
  /** Parent session for subagents and coordinator threads. NULL for top-level sessions. */
  parent_session_id?: string | null;
}
