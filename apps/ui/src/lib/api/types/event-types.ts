// Event types - SSE notifications following standard event protocol

import type { Message } from "./message-types";
import type { TokenUsage } from "./common-types";

/** Event context for correlation */
export interface EventContext {
  turn_id?: string;
  input_message_id?: string;
  exec_id?: string;
}

/** Standard event schema matching core::Event */
export interface Event {
  id: string;
  /** Event type using dot notation (e.g., "input.message", "tool.completed") */
  type: string;
  /** ISO timestamp */
  ts: string;
  session_id: string;
  context: EventContext;
  /** Event-specific payload. Schema depends on event type. */
  data: EventData;
  metadata?: Record<string, unknown>;
  tags?: string[];
  sequence?: number;
}

/** Durable user notification */
export interface Notification {
  id: string;
  kind: string;
  title: string;
  body: string;
  target_type?: string | null;
  target_id?: string | null;
  href?: string | null;
  payload: Record<string, unknown>;
  occurrence_count: number;
  viewed_at?: string | null;
  created_at: string;
  updated_at: string;
}

/** Notification list response with an accurate bell counter */
export interface ListNotificationsResponse {
  data: Notification[];
  unviewed_count: number;
}

// ============================================
// Event Data Types - Typed payloads for each event type
// ============================================

/** Model metadata for generation events */
export interface ModelMetadata {
  model: string;
  model_id?: string;
  provider_id?: string;
}

/** Data for input.message event */
export interface InputMessageData {
  message: Message;
}

/** Data for output.message.started event (LLM generation started) */
export interface OutputMessageStartedData {
  turn_id: string;
  model?: string;
  /** 1-based iteration number within the turn */
  iteration?: number;
}

/** Data for output.message.delta event (streaming text update) */
export interface OutputMessageDeltaData {
  turn_id: string;
  /** The new text since last delta */
  delta: string;
  /** Accumulated text so far */
  accumulated: string;
}

/** Data for output.message.completed event */
export interface OutputMessageCompletedData {
  message: Message;
  metadata?: ModelMetadata;
  usage?: TokenUsage;
}

/** Data for turn.started event */
export interface TurnStartedData {
  turn_id: string;
  input_message_id: string;
  /** Input message content (for observability) */
  input_content?: string;
}

/** Data for turn.completed event */
export interface TurnCompletedData {
  turn_id: string;
  iterations: number;
  duration_ms?: number;
  /** Aggregated token usage for all LLM calls in this turn */
  usage?: TokenUsage;
  /** Input message content (for observability, passed through from turn.started) */
  input_content?: string;
}

/** Data for turn.failed event */
export interface TurnFailedData {
  turn_id: string;
  error: string;
  error_code?: string;
}

/** Data for reason.started event */
export interface ReasonStartedData {
  agent_id: string;
  metadata?: ModelMetadata;
}

/** Data for reason.completed event */
export interface ReasonCompletedData {
  success: boolean;
  text_preview?: string;
  has_tool_calls: boolean;
  tool_call_count: number;
  error?: string;
  duration_ms?: number;
  usage?: TokenUsage;
}

/** Tool call summary (compact form) */
export interface ToolCallSummary {
  id: string;
  name: string;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  /** Human-readable narration for timeline rendering */
  narration?: string;
}

/** Data for act.started event */
export interface ActStartedData {
  tool_calls: ToolCallSummary[];
  headline?: string;
}

/** Data for act.completed event */
export interface ActCompletedData {
  completed: boolean;
  success_count: number;
  error_count: number;
  duration_ms?: number;
  headline?: string;
}

/** Tool call from LLM response */
export interface ToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

/** Data for tool.started event */
export interface ToolStartedData {
  tool_call: ToolCall;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  /** Human-readable narration for timeline rendering */
  narration?: string;
}

/** Data for tool.call_requested event (client-side tool calls awaiting results) */
export interface ToolCallRequestedData {
  tool_calls: Array<{
    id: string;
    name: string;
    arguments: Record<string, unknown>;
  }>;
  tool_summaries?: ToolCallSummary[];
  headline?: string;
}

/** Data for tool.progress event (interim status during execution) */
export interface ToolProgressData {
  tool_call_id: string;
  tool_name: string;
  /** Human-readable status message (e.g., "Connecting to browser…") */
  message: string;
  display_name?: string;
}

/** Data for tool.completed event */
export interface ToolCompletedData {
  tool_call_id: string;
  tool_name: string;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  success: boolean;
  status: "success" | "error" | "timeout" | "cancelled";
  result?: import("./message-types").ContentPart[];
  error?: string;
  duration_ms?: number;
  narration?: string;
}

/** LLM generation output */
export interface LlmGenerationOutput {
  text?: string;
  tool_calls: ToolCall[];
}

/** Information about context compaction during LLM generation */
export interface LlmCompactionInfo {
  compacted: boolean;
  input_tokens_before?: number;
  input_tokens_after?: number;
  duration_ms?: number;
}

/** LLM generation metadata */
export interface LlmGenerationMetadata {
  model: string;
  provider?: string;
  usage?: TokenUsage;
  duration_ms?: number;
  time_to_first_token_ms?: number;
  success: boolean;
  error?: string;
  compaction?: LlmCompactionInfo;
}

/** Summary of a tool definition available to the LLM */
export interface ToolDefinitionSummary {
  name: string;
  display_name?: string;
  description: string;
}

/** Data for llm.generation event */
export interface LlmGenerationData {
  messages: Message[];
  tools?: ToolDefinitionSummary[];
  output: LlmGenerationOutput;
  metadata: LlmGenerationMetadata;
}

/** Data for session.started event */
export interface SessionStartedData {
  agent_id: string;
  model_id?: string;
}

/** Data for session.activated event (turn started, session now active) */
export interface SessionActivatedData {
  turn_id: string;
  input_message_id: string;
}

/** Data for session.idled event (turn completed, session now idle) */
export interface SessionIdledData {
  turn_id: string;
  iterations?: number;
  /** Cumulative token usage for the session at this point */
  usage?: TokenUsage;
}

/** Data for reason.thinking.started event (LLM generation with thinking started) */
export interface ReasonThinkingStartedData {
  turn_id: string;
  model?: string;
}

/** Data for reason.thinking.delta event (streaming reasoning from extended thinking models) */
export interface ReasonThinkingDeltaData {
  turn_id: string;
  /** The new thinking text since last delta */
  delta: string;
  /** Accumulated thinking text so far */
  accumulated: string;
}

/** Data for reason.thinking.completed event (extended thinking completed) */
export interface ReasonThinkingCompletedData {
  turn_id: string;
  /** Complete thinking content */
  thinking: string;
}

/** A single step in a compaction cascade */
export interface CompactionStepData {
  strategy: string;
  messages_after: number;
  duration_ms: number;
}

/** Data for context.compacting event (compaction starting) */
export interface ContextCompactingData {
  reason: "proactive_budget" | "request_too_large" | "manual";
  strategy: string;
  messages_before: number;
}

/** Data for context.compacted event (compaction completed) */
export interface ContextCompactedData {
  strategy_used: string;
  messages_before: number;
  messages_after: number;
  duration_ms: number;
  steps: CompactionStepData[];
}

/** Per-session compaction metrics */
export interface SessionCompactionMetrics {
  compaction_count: number;
  total_messages_saved: number;
  strategy_counts: Record<string, number>;
  total_duration_ms: number;
}

/** Memory tier classification */
export type MemoryTier = "hot" | "warm" | "cold";

/** Union type for all event data types */
export type EventData =
  | InputMessageData
  | OutputMessageStartedData
  | OutputMessageDeltaData
  | OutputMessageCompletedData
  | TurnStartedData
  | TurnCompletedData
  | TurnFailedData
  | ReasonStartedData
  | ReasonCompletedData
  | ActStartedData
  | ActCompletedData
  | ToolStartedData
  | ToolCallRequestedData
  | ToolProgressData
  | ToolCompletedData
  | LlmGenerationData
  | SessionStartedData
  | SessionActivatedData
  | SessionIdledData
  | ReasonThinkingStartedData
  | ReasonThinkingDeltaData
  | ReasonThinkingCompletedData
  | ContextCompactingData
  | ContextCompactedData
  | Record<string, unknown>; // Raw/unknown event data

// ============================================
// Event type guard helpers
// ============================================
// These narrow Event.data to the correct payload type based on Event.type,
// replacing unsafe `as` casts with runtime-checked type predicates.

/** Map from event type string to its data type */
export interface EventTypeMap {
  "input.message": InputMessageData;
  "output.message.started": OutputMessageStartedData;
  "output.message.delta": OutputMessageDeltaData;
  "output.message.completed": OutputMessageCompletedData;
  "turn.started": TurnStartedData;
  "turn.completed": TurnCompletedData;
  "turn.failed": TurnFailedData;
  "reason.started": ReasonStartedData;
  "reason.completed": ReasonCompletedData;
  "act.started": ActStartedData;
  "act.completed": ActCompletedData;
  "tool.started": ToolStartedData;
  "tool.call_requested": ToolCallRequestedData;
  "tool.progress": ToolProgressData;
  "tool.completed": ToolCompletedData;
  "llm.generation": LlmGenerationData;
  "session.started": SessionStartedData;
  "session.activated": SessionActivatedData;
  "session.idled": SessionIdledData;
  "reason.thinking.started": ReasonThinkingStartedData;
  "reason.thinking.delta": ReasonThinkingDeltaData;
  "reason.thinking.completed": ReasonThinkingCompletedData;
  "context.compacting": ContextCompactingData;
  "context.compacted": ContextCompactedData;
}

export type KnownEventType = keyof EventTypeMap;

/** Type predicate: narrows an Event to one with a known type and correctly-typed data. */
export function isEventOfType<T extends KnownEventType>(
  event: Event,
  type: T,
): event is Event & { type: T; data: EventTypeMap[T] } {
  return event.type === type;
}

/** Narrow event data after a switch/if on event.type (returns typed data or null). */
export function getEventData<T extends KnownEventType>(
  event: Event,
  type: T,
): EventTypeMap[T] | null {
  return event.type === type ? (event.data as EventTypeMap[T]) : null;
}

/**
 * Type guard: narrows an unknown object value to Record<string, unknown>.
 * Use after checking `typeof value === 'object' && value !== null && !Array.isArray(value)`.
 */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Type guard for ToolCallContent shape (tool-call-card, tool-activity-group).
 * Validates the essential fields at runtime.
 */
export function isToolCallContent(value: unknown): value is {
  id: string;
  name: string;
  display_name?: string;
  arguments: Record<string, unknown>;
} {
  return (
    isRecord(value) &&
    typeof value.id === "string" &&
    typeof value.name === "string" &&
    isRecord(value.arguments)
  );
}

/**
 * Type guard for ToolResultContent shape.
 */
export function isToolResultContent(value: unknown): value is {
  tool_call_id: string;
  result?: unknown;
  error?: string;
} {
  return isRecord(value) && typeof value.tool_call_id === "string";
}

export interface CreateEventRequest {
  event_type: string;
  data: Record<string, unknown>;
}
