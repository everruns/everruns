// TypeScript types mirroring Rust types from everruns-core
// M2: Agent/Session/Messages model with Events as SSE notifications

// ============================================
// Agent types (M2)
// ============================================

export type AgentStatus = "active" | "archived";

export interface Agent {
  id: string;
  name: string;
  description: string | null;
  system_prompt: string;
  default_model_id: string | null;
  tags: string[];
  status: AgentStatus;
  created_at: string;
  updated_at: string;
}

export interface CreateAgentRequest {
  name: string;
  description?: string;
  system_prompt: string;
  default_model_id?: string;
  tags?: string[];
}

export interface UpdateAgentRequest {
  name?: string;
  description?: string;
  system_prompt?: string;
  default_model_id?: string;
  tags?: string[];
  status?: AgentStatus;
}

// ============================================
// Session types (M2)
// ============================================

export type SessionStatus = "pending" | "running" | "completed" | "failed";

export interface Session {
  id: string;
  agent_id: string;
  title: string | null;
  tags: string[];
  model_id: string | null;
  status: SessionStatus;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
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

export type MessageRole = "user" | "assistant" | "tool_call" | "tool_result" | "system";

// ContentPart discriminated union - message content parts
export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image"; url?: string; base64?: string; media_type?: string }
  | { type: "tool_call"; id: string; name: string; arguments: Record<string, unknown> }
  | { type: "tool_result"; result?: unknown; error?: string };

// Helper type guards for ContentPart
export function isTextPart(part: ContentPart): part is { type: "text"; text: string } {
  return part.type === "text";
}

export function isToolCallPart(part: ContentPart): part is { type: "tool_call"; id: string; name: string; arguments: Record<string, unknown> } {
  return part.type === "tool_call";
}

export function isToolResultPart(part: ContentPart): part is { type: "tool_result"; result?: unknown; error?: string } {
  return part.type === "tool_result";
}

// Reasoning configuration for model controls
export interface ReasoningConfig {
  effort?: string;
}

// Runtime controls for message processing
export interface Controls {
  model_id?: string;
  reasoning?: ReasoningConfig;
  max_tokens?: number;
  temperature?: number;
}

// Message response from API
export interface Message {
  id: string;
  session_id: string;
  sequence: number;
  role: MessageRole;
  content: ContentPart[];
  metadata?: Record<string, unknown>;
  tool_call_id: string | null;
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
// Event types (M2) - SSE notifications
// ============================================

export interface Event {
  id: string;
  session_id: string;
  sequence: number;
  event_type: string;
  data: Record<string, unknown>;
  created_at: string;
}

export interface CreateEventRequest {
  event_type: string;
  data: Record<string, unknown>;
}

// ============================================
// List response wrapper
// ============================================

export interface ListResponse<T> {
  data: T[];
}

// ============================================
// Tool types
// ============================================

export type ToolPolicy = "auto" | "requires_approval";

export type ToolDefinition = BuiltinTool;

export interface BuiltinTool {
  type: "builtin";
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  kind: "http_get" | "http_post" | "read_file" | "write_file" | "current_time";
  policy?: ToolPolicy;
}

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

export interface UserInfoResponse {
  id: string;
  email: string;
  name: string;
  roles: string[];
  avatar_url?: string;
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
  | "anthropic"
  | "azure_openai";

export type LlmProviderStatus = "active" | "disabled";
export type LlmModelStatus = "active" | "disabled";

export interface LlmProvider {
  id: string;
  name: string;
  provider_type: LlmProviderType;
  base_url?: string;
  api_key_set: boolean;
  is_default: boolean;
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
  is_default?: boolean;
}

export interface UpdateLlmProviderRequest {
  name?: string;
  provider_type?: LlmProviderType;
  base_url?: string;
  api_key?: string;
  is_default?: boolean;
  status?: LlmProviderStatus;
}

export interface CreateLlmModelRequest {
  model_id: string;
  display_name: string;
  capabilities?: string[];
  is_default?: boolean;
}

export interface UpdateLlmModelRequest {
  model_id?: string;
  display_name?: string;
  capabilities?: string[];
  is_default?: boolean;
  status?: LlmModelStatus;
}

// ============================================
// Capability types
// ============================================

export type CapabilityId = "noop" | "current_time" | "research" | "sandbox" | "file_system";

export type CapabilityStatus = "available" | "coming_soon" | "deprecated";

export interface Capability {
  id: CapabilityId;
  name: string;
  description: string;
  status: CapabilityStatus;
  icon?: string;
  category?: string;
}

export interface AgentCapability {
  capability_id: CapabilityId;
  position: number;
}

export interface UpdateAgentCapabilitiesRequest {
  capabilities: CapabilityId[];
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
