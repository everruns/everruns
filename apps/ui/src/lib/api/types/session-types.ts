// Session, Schedule, and Leased Resource types

import type {
  InitialFile,
  NetworkAccessList,
  PrincipalSummary,
  ToolDefinition,
  TokenUsage,
} from "./common-types";

// ============================================
// Session types (M2)
// ============================================

// Session status values:
// - "started": Session just created, no turn executed yet
// - "active": A turn is currently running
// - "idle": Turn completed, session waiting for next input
// - "waiting_for_tool_results": Session paused, waiting for client-side tool results
export type SessionStatus = "started" | "active" | "idle" | "waiting_for_tool_results";

export interface Session {
  id: string;
  /** Organization this session belongs to */
  organization_id: string;
  harness_id: string;
  agent_id: string | null;
  agent_identity_id?: string | null;
  owner_principal_id: string;
  resolved_owner_user_id?: string | null;
  owner?: PrincipalSummary | null;
  effective_owner?: PrincipalSummary | null;
  title: string | null;
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
  created_at: string;
  updated_at: string;
  started_at: string | null;
  finished_at: string | null;
  /** Cumulative token usage for all LLM calls in this session */
  usage?: TokenUsage;
  /** Whether this session is pinned by the current user */
  is_pinned?: boolean;
  /** Number of active schedules for this session */
  active_schedule_count?: number;
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
}

/** Session counts grouped by status */
export interface SessionStats {
  total: number;
  active: number;
  idle: number;
  started: number;
  waiting_for_tool_results: number;
}

/** Aggregate usage stats for an agent or harness detail page. */
export interface ResourceStats {
  session_count: number;
  active_session_count: number;
  idle_session_count: number;
  started_session_count: number;
  waiting_for_tool_results_session_count: number;
  execution_count: number;
  total_session_duration_ms: number;
  avg_session_duration_ms?: number | null;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cache_creation_tokens: number;
  total_actual_cost_usd: number;
  total_estimated_cost_usd: number;
  total_cost_usd: number;
  first_session_at?: string | null;
  last_session_at?: string | null;
  last_execution_at?: string | null;
}

export interface CreateSessionRequest {
  /** Harness ID for this session. If omitted, org base harness is used. */
  harness_id?: string;
  /** Agent ID to work in this session (optional) */
  agent_id?: string;
  agent_identity_id?: string;
  title?: string;
  locale?: string;
  tags?: string[];
  model_id?: string;
  /** Session-level system prompt override (prepended to agent prompt) */
  system_prompt?: string | null;
  /** Session-level initial files (additive to agent initial_files) */
  initial_files?: InitialFile[];
  /**
   * Session-level client hints -- arbitrary key-value pairs that tell the
   * server what the client can handle. Per-message `controls.hints` override
   * these key-by-key (shallow merge).
   */
  hints?: Record<string, unknown>;
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList;
}

export interface UpdateSessionRequest {
  title?: string;
  locale?: string;
  tags?: string[];
  model_id?: string;
}

// ============================================
// Session Schedule types
// ============================================

export type ScheduleType = "oneshot" | "recurring";

export interface SessionSchedule {
  id: string;
  session_id: string;
  owner_principal_id: string;
  resolved_owner_user_id?: string | null;
  owner?: PrincipalSummary | null;
  effective_owner?: PrincipalSummary | null;
  description: string;
  cron_expression?: string;
  scheduled_at?: string;
  timezone: string;
  enabled: boolean;
  schedule_type: ScheduleType;
  next_trigger_at?: string;
  last_triggered_at?: string;
  trigger_count: number;
  created_at: string;
  updated_at: string;
}

export interface UpdateSessionScheduleRequest {
  enabled?: boolean;
}

// ============================================
// Session Resource Registry types
// ============================================

export type SessionResourceStatus = "active" | "completed" | "failed" | "released";

/** A resource registered in the session resource registry. */
export interface SessionResourceEntry {
  /** Caller-provided stable ID (unique per session). */
  resource_id: string;
  session_id: string;
  /** Resource kind: "sandbox", "subagent", "browser_session", etc. */
  kind: string;
  /** Human-readable label. */
  display_name: string;
  status: SessionResourceStatus;
  /** Kind-specific non-secret metadata. */
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

// ============================================
// Session Task types
// ============================================

// Lifecycle state of a session task. Three classes: active (queued, running),
// interrupted (awaiting_input, resumable), terminal (succeeded, failed,
// canceled). Timeout/rejection are error.kind values on "failed", not states.
export type SessionTaskState =
  | "queued"
  | "running"
  | "awaiting_input"
  | "succeeded"
  | "failed"
  | "canceled";

/** True when the task state is terminal (succeeded, failed, canceled). */
export function isTerminalTaskState(state: SessionTaskState): boolean {
  return state === "succeeded" || state === "failed" || state === "canceled";
}

/** Progress shape shared with background tool execution. */
export interface TaskProgress {
  current?: number;
  total?: number;
  unit?: string;
  label?: string;
}

/** Structured ask posted by a task that needs input to continue. */
export interface TaskInputRequest {
  /** Stable ID referenced by the answering message's `in_reply_to`. */
  id: string;
  /** Human/agent-readable prompt. */
  prompt: string;
  /** Optional machine-readable description of the expected answer. */
  expected?: unknown;
}

/** Terminal error detail. Timeout/rejection/orphaned are kinds, not states. */
export interface TaskError {
  kind: string;
  message: string;
}

/** Typed link to something the task produced. */
export interface TaskArtifact {
  name: string;
  /** Artifact type: "file", "url", "session", "pr", etc. */
  type: string;
  /** Session VFS path, when the artifact lives in the session filesystem. */
  path?: string;
  /** External URL, when the artifact lives elsewhere. */
  url?: string;
}

/** Cross-references owned by a task. */
export interface TaskLinks {
  /** Child session, for subagent-shaped tasks. Full transcript lives there. */
  child_session_id?: string;
  /** Remote task ID, for tasks wrapping an external protocol task (A2A). */
  remote_task_id?: string;
  /** Session resources (sandboxes, browser sessions) this task holds. */
  resource_ids?: string[];
}

/** When outbound task activity wakes the owning session's agent. */
export type TaskWakePolicy = "silent" | "on_terminal" | "on_activity";

/** A unit of background work owned by a session. */
export interface SessionTask {
  /** `task_*` public ID. */
  id: string;
  session_id: string;
  /** Task kind: "subagent", "external_agent", "background_tool", "monitor", etc. */
  kind: string;
  /** Human-readable label. */
  display_name: string;
  /** Kind-specific input (instructions, tool args, external agent id). */
  spec: unknown;
  state: SessionTaskState;
  /** Short live status line ("polling remote task", "iteration 4/10"). */
  state_detail?: string;
  progress?: TaskProgress;
  /** Pending ask while `awaiting_input`; cleared when answered. */
  input_request?: TaskInputRequest;
  /** Cooperative cancel intent. A flag, not a state. */
  cancel_requested_at?: string;
  /** Human-readable outcome. */
  summary?: string;
  /** Machine result in the session VFS: `/.tasks/{task_id}/result.json`. */
  result_path?: string;
  artifacts?: TaskArtifact[];
  error?: TaskError;
  /** Execution attempt, starting at 1. Incremented on re-attach. */
  attempt: number;
  worker_id?: string;
  heartbeat_at?: string;
  links?: TaskLinks;
  wake_policy: TaskWakePolicy;
  created_at: string;
  started_at?: string;
  finished_at?: string;
  updated_at: string;
}

/** One content part of a task message. */
export type TaskMessagePart = { type: "text"; text: string } | { type: "data"; data: unknown };

/** A message exchanged between a session and one of its tasks. */
export interface TaskMessage {
  /** `tmsg_*` public ID. */
  id: string;
  task_id: string;
  /** Inbound = session to task. */
  direction: "inbound" | "outbound";
  content: TaskMessagePart[];
  /** Set when this message answers a `TaskInputRequest`. */
  in_reply_to?: string;
  created_at: string;
}

/** Task snapshot plus its recent message thread. */
export interface SessionTaskDetail {
  task: SessionTask;
  messages: TaskMessage[];
}

/** Request to post an inbound message (steering or input answer) to a task. */
export interface PostTaskMessageRequest {
  content: TaskMessagePart[];
  in_reply_to?: string;
}

// ============================================
// Session Storage types (Key-Value & Secrets)
// ============================================

/** Key-value entry info */
export interface KeyValueInfo {
  /** The key name */
  key: string;
  /** The stored value */
  value: string;
  /** When the key was created */
  created_at: string;
  /** When the key was last updated */
  updated_at: string;
}

/** Secret entry info (no value exposed) */
export interface SecretInfo {
  /** The secret name */
  name: string;
  /** When the secret was created */
  created_at: string;
  /** When the secret was last updated */
  updated_at: string;
}

// ============================================
// Session File types (Virtual Filesystem)
// ============================================

/** File metadata without content */
export interface FileInfo {
  id: string;
  session_id: string;
  path: string;
  name: string;
  is_directory: boolean;
  is_readonly: boolean;
  size_bytes: number;
  created_at: string;
  updated_at: string;
}

/** Full file with content */
export interface SessionFile extends FileInfo {
  /** File content (text or base64 encoded) */
  content?: string;
  /** Content encoding: "text" or "base64" */
  encoding: string;
}

/** File stat information */
export interface FileStat {
  path: string;
  name: string;
  is_directory: boolean;
  is_readonly: boolean;
  size_bytes: number;
  created_at: string;
  updated_at: string;
}

/** Grep match in a single line */
export interface GrepMatch {
  path: string;
  line_number: number;
  line: string;
}

/** Grep results for a file */
export interface GrepResult {
  path: string;
  matches: GrepMatch[];
}

/** Request to create a file or directory */
export interface CreateFileRequest {
  path: string;
  content?: string;
  encoding?: string;
  is_readonly?: boolean;
  /** Set to true to create a directory instead of a file */
  is_directory?: boolean;
}

/** Request to update a file */
export interface UpdateFileRequest {
  content?: string;
  encoding?: string;
  is_readonly?: boolean;
}

/** Request to move/rename a file */
export interface MoveFileRequest {
  src_path: string;
  dst_path: string;
}

/** Request to copy a file */
export interface CopyFileRequest {
  src_path: string;
  dst_path: string;
}

/** Request to search files */
export interface GrepRequest {
  pattern: string;
  path_pattern?: string;
}

/** Delete response */
export interface DeleteFileResponse {
  deleted: boolean;
}
