// TypeScript types mirroring Rust contracts from everruns-contracts
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

export interface Message {
  id: string;
  session_id: string;
  sequence: number;
  role: MessageRole;
  content: Record<string, unknown>;
  tool_call_id: string | null;
  created_at: string;
}

export interface CreateMessageRequest {
  role: MessageRole;
  content: Record<string, unknown>;
  tool_call_id?: string;
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

export type ToolDefinition = WebhookTool | BuiltinTool;

export interface WebhookTool {
  type: "webhook";
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  url: string;
  method?: string;
  headers?: Record<string, string>;
  signing_secret?: string;
  timeout_secs?: number;
  max_retries?: number;
  policy?: ToolPolicy;
}

export interface BuiltinTool {
  type: "builtin";
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  kind: "http_get" | "http_post" | "read_file" | "write_file";
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
// LLM Provider types
// ============================================

export type LlmProviderType =
  | "openai"
  | "anthropic"
  | "azure_openai"
  | "ollama"
  | "custom";

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
  context_window?: number;
  is_default: boolean;
  status: LlmModelStatus;
  created_at: string;
  updated_at: string;
}

export interface LlmModelWithProvider extends LlmModel {
  provider_name: string;
  provider_type: LlmProviderType;
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
  context_window?: number;
  is_default?: boolean;
}

export interface UpdateLlmModelRequest {
  model_id?: string;
  display_name?: string;
  capabilities?: string[];
  context_window?: number;
  is_default?: boolean;
  status?: LlmModelStatus;
}
