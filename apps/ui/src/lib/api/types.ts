// TypeScript types mirroring Rust contracts from everruns-contracts

// Agent types
export type AgentStatus = "active" | "disabled";

export interface Agent {
  id: string;
  name: string;
  description: string | null;
  default_model_id: string;
  definition: AgentDefinition;
  status: AgentStatus;
  created_at: string;
  updated_at: string;
}

export interface AgentDefinition {
  system_prompt?: string;
  temperature?: number;
  max_tokens?: number;
  tools?: ToolDefinition[];
}

export interface LlmConfig {
  model?: string;
  temperature?: number;
  max_tokens?: number;
  top_p?: number;
}

// Tool types
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

// Thread types
export interface Thread {
  id: string;
  created_at: string;
}

export interface Message {
  id: string;
  thread_id: string;
  role: string;
  content: string;
  metadata: Record<string, unknown> | null;
  created_at: string;
}

// Run types
export type RunStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

export interface Run {
  id: string;
  agent_id: string;
  thread_id: string;
  status: RunStatus;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
}

// AG-UI Event types
export type AgUiEvent =
  | { type: "RUN_STARTED"; run_id: string; timestamp: string }
  | { type: "RUN_FINISHED"; run_id: string; timestamp: string }
  | { type: "RUN_ERROR"; run_id: string; error: string; timestamp: string }
  | {
      type: "STEP_STARTED";
      step_id: string;
      step_name: string;
      timestamp: string;
    }
  | { type: "STEP_FINISHED"; step_id: string; timestamp: string }
  | {
      type: "TEXT_MESSAGE_START";
      message_id: string;
      role: string;
      timestamp: string;
    }
  | {
      type: "TEXT_MESSAGE_CHUNK";
      message_id: string;
      chunk: string;
      timestamp: string;
    }
  | { type: "TEXT_MESSAGE_END"; message_id: string; timestamp: string }
  | {
      type: "TOOL_CALL_START";
      tool_call_id: string;
      tool_name: string;
      arguments: Record<string, unknown>;
      timestamp: string;
    }
  | {
      type: "TOOL_CALL_RESULT";
      tool_call_id: string;
      result: Record<string, unknown> | null;
      error: string | null;
      timestamp: string;
    }
  | {
      type: "CUSTOM";
      kind: string;
      data: Record<string, unknown>;
      timestamp: string;
    };

// API Request types
export interface CreateAgentRequest {
  name: string;
  description?: string;
  default_model_id: string;
  definition: AgentDefinition;
}

export interface UpdateAgentRequest {
  name?: string;
  description?: string;
  default_model_id?: string;
  definition?: AgentDefinition;
  status?: AgentStatus;
}

// Empty request body - threads are created without parameters
export type CreateThreadRequest = Record<string, never>;

export interface CreateMessageRequest {
  role: string;
  content: string;
  metadata?: Record<string, unknown>;
}

export interface CreateRunRequest {
  agent_id: string;
  thread_id: string;
}

// Health check
export interface HealthResponse {
  status: string;
  version: string;
}

// LLM Provider types
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
