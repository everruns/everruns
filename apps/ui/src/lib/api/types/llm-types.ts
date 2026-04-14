// LLM Provider and Model types

// ============================================
// LLM Provider types
// ============================================

export type LlmProviderType = "openai" | "openai_completions" | "anthropic" | "gemini";

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
  enabled: boolean;
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
  /** Tiered pricing above certain context thresholds */
  cost_tiers?: CostTier[];
}

/** A pricing tier that activates above a context token threshold */
export interface CostTier {
  /** Context token threshold above which this tier applies */
  above_tokens: number;
  /** Input cost per million tokens (USD) for this tier */
  input: number;
  /** Output cost per million tokens (USD) for this tier */
  output: number;
  /** Cached read cost per million tokens (USD) for this tier, if supported */
  cache_read?: number;
}

/** Token limits for the model */
export interface LlmModelLimits {
  /** Maximum context window size in tokens */
  context: number;
  /** Maximum input tokens (if different from context - output) */
  input?: number;
  /** Maximum output tokens */
  output: number;
  /** Maximum images or PDF pages per request */
  max_media?: number;
}

/** Modality type */
export type Modality = "text" | "image" | "audio" | "video" | "pdf";

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
  /** Short human-readable description of the model's strengths and intended use */
  description?: string;
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
  enabled?: boolean;
  is_favorite?: boolean;
}

export interface UpdateLlmModelRequest {
  model_id?: string;
  display_name?: string;
  capabilities?: string[];
  enabled?: boolean;
  is_favorite?: boolean;
  status?: LlmModelStatus;
}

// Response from syncing models from a provider
export type SyncModelsResponse =
  | { status: "success"; created: number; updated: number; stale: number }
  | { status: "not_supported" };
