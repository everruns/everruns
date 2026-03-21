// Session, Schedule, and Leased Resource types

import type { ToolDefinition, TokenUsage } from "./common-types";

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
  /** Session-level client hints (defaults for every turn) */
  hints?: Record<string, unknown>;
}

/** Session counts grouped by status */
export interface SessionStats {
  total: number;
  active: number;
  idle: number;
  started: number;
  waiting_for_tool_results: number;
}

export interface CreateSessionRequest {
  /** Harness ID for this session. If omitted, org base harness is used. */
  harness_id?: string;
  /** Agent ID to work in this session (optional) */
  agent_id?: string;
  /** Resident agent identity for unattended/background execution */
  agent_identity_id?: string;
  title?: string;
  locale?: string;
  tags?: string[];
  model_id?: string;
  /**
   * Session-level client hints -- arbitrary key-value pairs that tell the
   * server what the client can handle. Per-message `controls.hints` override
   * these key-by-key (shallow merge).
   */
  hints?: Record<string, unknown>;
}

export interface UpdateSessionRequest {
  title?: string;
  agent_identity_id?: string | null;
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
// Session Leased Resource types
// ============================================

export type LeasedResourceStatus = "active" | "cleaning" | "released" | "cleanup_failed";

export interface LeasedResource {
  id: string;
  session_id?: string;
  provider: string;
  resource_type: string;
  external_id: string;
  display_name?: string;
  status: LeasedResourceStatus;
  owner_user_id?: string;
  lease_duration_seconds: number;
  last_touched_at: string;
  lease_expires_at: string;
  cleanup_started_at?: string;
  cleanup_completed_at?: string;
  cleanup_attempts: number;
  last_cleanup_error?: string;
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
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
