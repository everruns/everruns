// TypeScript types mirroring Rust types from everruns-core
// M2: Agent/Session/Messages model with Events as SSE notifications

// ============================================
// Agent types (M2)
// ============================================

export type AgentStatus = "active" | "archived";

/** Capability ID - extensible string-based identifier */
export type CapabilityId = string;

/** Per-agent capability configuration */
export interface AgentCapabilityConfig {
  /** Reference to the capability ID */
  ref: CapabilityId;
  /** Per-agent configuration for this capability (capability-specific) */
  config: Record<string, unknown>;
}

export interface Agent {
  id: string;
  name: string;
  description: string | null;
  system_prompt: string;
  default_model_id: string | null;
  tags: string[];
  /** Capabilities with per-agent configuration */
  capabilities: AgentCapabilityConfig[];
  status: AgentStatus;
  created_at: string;
  updated_at: string;
  /** Cumulative token usage across all sessions for this agent */
  usage?: TokenUsage;
}

export interface CreateAgentRequest {
  name: string;
  description?: string;
  system_prompt: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
}

export interface UpdateAgentRequest {
  name?: string;
  description?: string;
  system_prompt?: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  status?: AgentStatus;
}

/** Request to preview the final agent shape with capabilities applied */
export interface PreviewAgentRequest {
  /** The base system prompt (before capability additions) */
  system_prompt: string;
  /** Capabilities to apply with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
}

/** Response showing the final agent shape after applying capabilities */
export interface AgentPreviewResponse {
  /** The full system prompt with capability additions prepended */
  system_prompt: string;
  /** All tool definitions from capabilities */
  tools: ToolDefinition[];
}

// ============================================
// Session types (M2)
// ============================================

// Session status values:
// - "started": Session just created, no turn executed yet
// - "active": A turn is currently running
// - "idle": Turn completed, session waiting for next input
export type SessionStatus = "started" | "active" | "idle";

export interface Session {
  id: string;
  agent_id: string;
  title: string | null;
  /** Preview text from the first user message (truncated) */
  preview?: string | null;
  /** Preview text from the last assistant response (truncated) */
  output_preview?: string | null;
  tags: string[];
  model_id: string | null;
  status: SessionStatus;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  /** Cumulative token usage for all LLM calls in this session */
  usage?: TokenUsage;
}

export interface CreateSessionRequest {
  title?: string;
  tags?: string[];
  model_id?: string;
}

export interface UpdateSessionRequest {
  title?: string;
  tags?: string[];
  model_id?: string;
}

// ============================================
// Message types (M2) - PRIMARY data
// ============================================

/**
 * Message role (API layer)
 *
 * Simplified to only user and agent messages.
 * Tool results are conveyed via `tool.completed` events.
 * System messages are internal and not exposed via API.
 */
export type MessageRole = "user" | "agent";

/**
 * Display message role (UI layer)
 *
 * Extended role type for rendering messages in the UI.
 * Includes "tool_result" for displaying tool execution results from events.
 */
export type DisplayMessageRole = MessageRole | "tool_result";

// ContentPart discriminated union - message content parts
export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image"; url?: string; base64?: string; media_type?: string }
  | { type: "image_file"; image_id: string; filename?: string }
  | { type: "tool_call"; id: string; name: string; arguments: Record<string, unknown> }
  | { type: "tool_result"; tool_call_id: string; result?: unknown; error?: string };

// Helper type guards for ContentPart
export function isTextPart(part: ContentPart): part is { type: "text"; text: string } {
  return part.type === "text";
}

export function isToolCallPart(part: ContentPart): part is { type: "tool_call"; id: string; name: string; arguments: Record<string, unknown> } {
  return part.type === "tool_call";
}

export function isToolResultPart(part: ContentPart): part is { type: "tool_result"; tool_call_id: string; result?: unknown; error?: string } {
  return part.type === "tool_result";
}

export function isImageFilePart(part: ContentPart): part is { type: "image_file"; image_id: string; filename?: string } {
  return part.type === "image_file";
}

// Reasoning configuration for model controls
export interface ReasoningConfig {
  effort?: string;
}

// Runtime controls for message processing
// Model resolution priority: controls.model_id > session.model_id > agent.default_model_id > system default
export interface Controls {
  /** UUID of the model to use for this message (overrides session/agent settings) */
  model_id?: string;
  reasoning?: ReasoningConfig;
  max_tokens?: number;
  temperature?: number;
}

/**
 * Message for UI display
 *
 * Uses DisplayMessageRole since messages can be derived from events
 * including tool.call_completed events which become "tool_result" messages.
 */
export interface Message {
  id: string;
  session_id: string;
  sequence: number;
  role: DisplayMessageRole;
  content: ContentPart[];
  metadata?: Record<string, unknown>;
  tool_call_id: string | null;
  /** Extended thinking content (Anthropic Claude with reasoning) */
  thinking?: string;
  /** Cryptographic signature for thinking (required for multi-turn) */
  thinking_signature?: string;
  created_at: string;
}

// Message input for creating a message
export interface MessageInput {
  role: MessageRole;
  content: ContentPart[];
  metadata?: Record<string, unknown>;
  tool_call_id?: string;
}

// Request to create a message (new contract)
export interface CreateMessageRequest {
  message: MessageInput;
  controls?: Controls;
  metadata?: Record<string, unknown>;
  tags?: string[];
}

// Helper function to create a simple text message request
export function createTextMessageRequest(text: string, controls?: Controls): CreateMessageRequest {
  return {
    message: {
      role: "user",
      content: [{ type: "text", text }],
    },
    controls,
  };
}

// Helper function to extract text from content parts
export function getTextFromContent(content: ContentPart[]): string {
  return content
    .filter(isTextPart)
    .map(part => part.text)
    .join("\n");
}

// Helper function to get tool calls from content parts
export function getToolCallsFromContent(content: ContentPart[]): Array<{ id: string; name: string; arguments: Record<string, unknown> }> {
  return content
    .filter(isToolCallPart)
    .map(part => ({ id: part.id, name: part.name, arguments: part.arguments }));
}

// ============================================
// Event types - SSE notifications following standard event protocol
// ============================================

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

// ============================================
// Event Data Types - Typed payloads for each event type
// ============================================

/** Model metadata for generation events */
export interface ModelMetadata {
  model: string;
  model_id?: string;
  provider_id?: string;
}

/** Token usage statistics */
export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  /** Tokens read from cache (reduces cost) */
  cache_read_tokens?: number;
  /** Tokens written to cache (Anthropic-specific) */
  cache_creation_tokens?: number;
}

/** Data for input.message event */
export interface InputMessageData {
  message: Message;
}

/** Data for output.message.started event (LLM generation started) */
export interface OutputMessageStartedData {
  turn_id: string;
  model?: string;
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
}

/** Data for act.started event */
export interface ActStartedData {
  tool_calls: ToolCallSummary[];
}

/** Data for act.completed event */
export interface ActCompletedData {
  completed: boolean;
  success_count: number;
  error_count: number;
  duration_ms?: number;
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
}

/** Data for tool.completed event */
export interface ToolCompletedData {
  tool_call_id: string;
  tool_name: string;
  success: boolean;
  status: "success" | "error" | "timeout" | "cancelled";
  result?: ContentPart[];
  error?: string;
  duration_ms?: number;
}

/** LLM generation output */
export interface LlmGenerationOutput {
  text?: string;
  tool_calls: ToolCall[];
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
}

/** Data for llm.generation event */
export interface LlmGenerationData {
  messages: Message[];
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
  | ToolCompletedData
  | LlmGenerationData
  | SessionStartedData
  | SessionActivatedData
  | SessionIdledData
  | ReasonThinkingStartedData
  | ReasonThinkingDeltaData
  | ReasonThinkingCompletedData
  | Record<string, unknown>; // Raw/unknown event data

export interface CreateEventRequest {
  event_type: string;
  data: Record<string, unknown>;
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
// Tool types
// ============================================

export type ToolPolicy = "auto" | "requires_approval";

/** Tool definition - builtin tool configuration */
export interface BuiltinTool {
  type: "builtin";
  /** Tool name (used by LLM and for registry lookup) */
  name: string;
  /** Tool description for LLM */
  description: string;
  /** JSON schema for tool parameters */
  parameters: Record<string, unknown>;
  /** Tool policy (auto or requires_approval) */
  policy?: ToolPolicy;
}

/** Tool definition - currently only supports builtin tools */
export type ToolDefinition = BuiltinTool;

// ============================================
// Health check
// ============================================

export interface HealthResponse {
  status: string;
  version: string;
  runner_mode: string;
}

// ============================================
// Authentication types
// ============================================

export type AuthMode = "none" | "admin" | "full";

export interface AuthConfigResponse {
  mode: AuthMode;
  password_auth_enabled: boolean;
  oauth_providers: string[];
  signup_enabled: boolean;
}

export interface LoginRequest {
  email: string;
  password: string;
}

export interface RegisterRequest {
  email: string;
  password: string;
  name: string;
}

export interface TokenResponse {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token?: string;
}

/** Organization membership info */
export interface OrganizationMembership {
  public_id: string;
  name: string;
}

export interface UserInfoResponse {
  id: string;
  email: string;
  name: string;
  roles: string[];
  avatar_url?: string;
  /** Organizations the user belongs to */
  organizations?: OrganizationMembership[];
}

export interface ApiKeyResponse {
  id: string;
  name: string;
  key: string;
  key_prefix: string;
  scopes: string[];
  expires_at?: string;
  created_at: string;
}

export interface ApiKeyListItem {
  id: string;
  name: string;
  key_prefix: string;
  scopes: string[];
  expires_at?: string;
  last_used_at?: string;
  created_at: string;
}

export interface CreateApiKeyRequest {
  name: string;
  scopes?: string[];
  expires_in_days?: number;
}

export interface RefreshTokenRequest {
  refresh_token: string;
}

// ============================================
// LLM Provider types
// ============================================

export type LlmProviderType =
  | "openai"
  | "openai_completions"
  | "anthropic";

export type LlmProviderStatus = "active" | "disabled";
export type LlmModelStatus = "active" | "disabled";

export interface LlmProvider {
  id: string;
  name: string;
  provider_type: LlmProviderType;
  base_url?: string;
  api_key_set: boolean;
  status: LlmProviderStatus;
  created_at: string;
  updated_at: string;
}

export interface LlmModel {
  id: string;
  provider_id: string;
  model_id: string;
  display_name: string;
  capabilities: string[];
  is_default: boolean;
  is_favorite: boolean;
  status: LlmModelStatus;
  created_at: string;
  updated_at: string;
}

export interface LlmModelWithProvider extends LlmModel {
  provider_name: string;
  provider_type: LlmProviderType;
  /** Readonly profile with model capabilities (not persisted to database) */
  profile?: LlmModelProfile;
}

// ============================================
// LLM Model Profile types
// Based on models.dev structure
// ============================================

/** Cost information for the model (per million tokens in USD) */
export interface LlmModelCost {
  /** Input cost per million tokens */
  input: number;
  /** Output cost per million tokens */
  output: number;
  /** Cached read cost per million tokens, if supported */
  cache_read?: number;
}

/** Token limits for the model */
export interface LlmModelLimits {
  /** Maximum context window size in tokens */
  context: number;
  /** Maximum output tokens */
  output: number;
}

/** Modality type */
export type Modality = "text" | "image" | "audio" | "video";

/** Model modalities for input and output */
export interface LlmModelModalities {
  /** Supported input modalities */
  input: Modality[];
  /** Supported output modalities */
  output: Modality[];
}

/** Reasoning effort level for models that support it */
export type ReasoningEffort = "none" | "minimal" | "low" | "medium" | "high" | "xhigh";

/** Named reasoning effort value for UI display */
export interface ReasoningEffortValue {
  /** The API value (e.g., "low", "medium") */
  value: ReasoningEffort;
  /** Display name (e.g., "Low", "Medium") */
  name: string;
}

/** Reasoning effort configuration for a model */
export interface ReasoningEffortConfig {
  /** Available reasoning effort values for this model */
  values: ReasoningEffortValue[];
  /** Default reasoning effort for this model */
  default: ReasoningEffort;
}

/**
 * LLM Model Profile describing model capabilities
 * Based on models.dev structure (https://models.dev/api.json)
 */
export interface LlmModelProfile {
  /** Display name of the model */
  name: string;
  /** Model family (e.g., "gpt-4o", "claude-3-5-sonnet") */
  family: string;
  /** Release date (YYYY-MM-DD format) */
  release_date?: string;
  /** Last updated date (YYYY-MM-DD format) */
  last_updated?: string;
  /** Whether the model supports file/image attachments */
  attachment: boolean;
  /** Whether the model has reasoning/chain-of-thought capabilities */
  reasoning: boolean;
  /** Whether temperature control is supported */
  temperature: boolean;
  /** Knowledge cutoff date (YYYY-MM-DD format) */
  knowledge?: string;
  /** Whether the model supports tool/function calling */
  tool_call: boolean;
  /** Whether the model supports structured output (JSON mode) */
  structured_output: boolean;
  /** Whether the model has open weights */
  open_weights: boolean;
  /** Cost per million tokens */
  cost?: LlmModelCost;
  /** Token limits */
  limits?: LlmModelLimits;
  /** Supported modalities */
  modalities?: LlmModelModalities;
  /** Reasoning effort configuration (for reasoning models) */
  reasoning_effort?: ReasoningEffortConfig;
}

export interface CreateLlmProviderRequest {
  name: string;
  provider_type: LlmProviderType;
  base_url?: string;
  api_key?: string;
}

export interface UpdateLlmProviderRequest {
  name?: string;
  provider_type?: LlmProviderType;
  base_url?: string;
  api_key?: string;
  status?: LlmProviderStatus;
}

export interface CreateLlmModelRequest {
  model_id: string;
  display_name: string;
  capabilities?: string[];
  is_default?: boolean;
  is_favorite?: boolean;
}

export interface UpdateLlmModelRequest {
  model_id?: string;
  display_name?: string;
  capabilities?: string[];
  is_default?: boolean;
  is_favorite?: boolean;
  status?: LlmModelStatus;
}

// Response from syncing models from a provider
export type SyncModelsResponse =
  | { status: "success"; created: number; updated: number; stale: number }
  | { status: "not_supported" };

// ============================================
// Capability types
// ============================================

// NOTE: CapabilityId is defined with Agent types above for proper ordering

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
}

// ============================================
// User types (for members management)
// ============================================

export interface User {
  id: string;
  email: string;
  name: string;
  avatar_url?: string;
  roles: string[];
  auth_provider?: string;
  created_at: string;
}

export interface ListUsersQuery {
  search?: string;
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

// ============================================
// Durable Execution types
// ============================================

/** Worker status */
export type WorkerStatus = "active" | "draining" | "stopped" | "stale";

/** Workflow status */
export type WorkflowStatus = "pending" | "running" | "completed" | "failed" | "cancelled";

/** Task status in the queue */
export type TaskStatus = "pending" | "claimed" | "completed" | "failed" | "dead" | "cancelled";

/** Circuit breaker state */
export type CircuitBreakerState = "closed" | "open" | "half_open";

/** System health status */
export type HealthStatus = "healthy" | "degraded" | "unhealthy";

/** Worker information */
export interface DurableWorker {
  id: string;
  worker_group: string;
  activity_types: string[];
  max_concurrency: number;
  current_load: number;
  status: WorkerStatus;
  accepting_tasks: boolean;
  backpressure_reason?: string;
  started_at: string;
  last_heartbeat_at: string;
  hostname?: string;
  version?: string;
  metadata?: Record<string, unknown>;
  // Live stats (optional - may not be tracked)
  tasks_completed?: number;
  tasks_failed?: number;
  avg_task_duration_ms?: number;
}

/** Workers list summary */
export interface WorkersSummary {
  active: number;
  draining: number;
  stopped: number;
  total_capacity: number;
  total_load: number;
}

/** Workers list response */
export interface WorkersResponse {
  data: DurableWorker[];
  total: number;
  summary?: WorkersSummary;
}

/** Workflow instance */
export interface DurableWorkflow {
  id: string;
  workflow_type: string;
  status: WorkflowStatus;
  input: Record<string, unknown>;
  result?: Record<string, unknown>;
  error?: Record<string, unknown>;
  created_at: string;
  started_at?: string;
  completed_at?: string;
  // Optional linked session
  session_id?: string;
  agent_id?: string;
}

/** Workflow event */
export interface WorkflowEvent {
  id: number;
  workflow_id: string;
  sequence_num: number;
  event_type: string;
  event_data: Record<string, unknown>;
  created_at: string;
}

/** Task in the queue */
export interface DurableTask {
  id: string;
  workflow_id: string;
  activity_id: string;
  activity_type: string;
  status: TaskStatus;
  priority: number;
  scheduled_at: string;
  visible_at: string;
  claimed_by?: string;
  claimed_at?: string;
  heartbeat_at?: string;
  attempt: number;
  max_attempts: number;
  last_error?: string;
  created_at: string;
}

/** Activity type statistics */
export interface ActivityTypeStats {
  pending: number;
  claimed: number;
  completed_last_hour: number;
  failed_last_hour: number;
  avg_duration_ms: number;
  p99_duration_ms: number;
}

/** Task queue statistics */
export interface TaskQueueStats {
  by_activity_type: Record<string, ActivityTypeStats>;
  by_priority: Record<number, number>;
  oldest_pending_task_age_ms: number;
  avg_schedule_to_start_ms: number;
  avg_execution_time_ms: number;
}

/** Dead letter queue entry */
export interface DlqEntry {
  id: string;
  original_task_id: string;
  workflow_id: string;
  activity_id: string;
  activity_type: string;
  input: Record<string, unknown>;
  attempts: number;
  last_error: string;
  error_history: string[];
  dead_at: string;
  requeued_at?: string;
  requeue_count: number;
}

/** Circuit breaker information */
export interface CircuitBreaker {
  key: string;
  state: CircuitBreakerState;
  failure_count: number;
  success_count: number;
  last_failure_at?: string;
  opened_at?: string;
  half_open_at?: string;
  updated_at: string;
}

/** Durable system health */
export interface DurableSystemHealth {
  status: HealthStatus;
  // Workers
  total_workers: number;
  active_workers: number;
  workers_accepting: number;
  total_capacity: number;
  current_load: number;
  load_percentage: number;
  // Task queue
  pending_tasks: number;
  claimed_tasks: number;
  queue_depth_by_type?: Record<string, number>;
  // Workflows
  running_workflows: number;
  pending_workflows: number;
  // DLQ
  dlq_size: number;
  // Circuit breakers (optional - computed from circuit-breakers endpoint if needed)
  open_circuit_breakers?: string[];
}

/** Workflows list response */
export interface WorkflowsResponse {
  data: DurableWorkflow[];
  total: number;
}

/** Tasks list response */
export interface TasksResponse {
  data: DurableTask[];
  total: number;
}

/** DLQ list response */
export interface DlqResponse {
  data: DlqEntry[];
  total: number;
}

/** Workflow events list response */
export interface WorkflowEventsResponse {
  data: WorkflowEvent[];
  total: number;
}

/** Circuit breakers list response */
export interface CircuitBreakersResponse {
  data: CircuitBreaker[];
  total: number;
}

// ============================================
// MCP Server types
// ============================================

/** MCP Server transport type */
export type McpServerTransportType = "http";

/** MCP Server status */
export type McpServerStatus = "active" | "disabled";

/** MCP Server configuration */
export interface McpServer {
  id: string;
  name: string;
  description: string | null;
  url: string;
  transport_type: McpServerTransportType;
  status: McpServerStatus;
  api_key_set: boolean;
  headers: Record<string, string>;
  created_at: string;
  updated_at: string;
}

/** Request to create an MCP server */
export interface CreateMcpServerRequest {
  name: string;
  description?: string;
  url: string;
  transport_type?: McpServerTransportType;
  api_key?: string;
  headers?: Record<string, string>;
}

/** Request to update an MCP server */
export interface UpdateMcpServerRequest {
  name?: string;
  description?: string;
  url?: string;
  transport_type?: McpServerTransportType;
  status?: McpServerStatus;
  api_key?: string;
  headers?: Record<string, string>;
}

// ============================================
// Image types (for message attachments)
// ============================================

/** Image metadata (returned from upload) */
export interface ImageInfo {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  metadata: Record<string, unknown>;
  created_at: string;
}

/** Image upload response */
export interface ImageUploadResponse {
  id: string;
  filename: string;
  content_type: string;
  size_bytes: number;
  created_at: string;
}

/** Allowed image content types */
export const ALLOWED_IMAGE_TYPES = [
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
] as const;

export type AllowedImageType = typeof ALLOWED_IMAGE_TYPES[number];

/** Maximum image size in bytes (100 MB) */
export const MAX_IMAGE_SIZE = 100 * 1024 * 1024;

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
