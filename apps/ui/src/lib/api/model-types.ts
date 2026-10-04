import type { DriverId } from "./provider-driver-types";
import type { ModelVendor } from "./legacy-api-types";
export type ModelService = "chat" | "decisions" | "embeddings" | "realtime" | "images" | "rerank";
export interface DecisionModelProfile {
  primitives: string[];
  calibrated: boolean;
  max_choice_options?: number | null;
  max_score_levels?: number | null;
  state_tokens?: number | null;
  request_tokens?: number | null;
}
export interface ModelProfileResponse {
  key: string;
  service: ModelService;
  vendor?: ModelVendor;
  source: string;
  profile: ModelProfile;
}

export interface Model {
  id: string;
  provider_id: string;
  model_id: string;
  service?: ModelService;
  profile_key?: string;
  display_name: string;
  capabilities: string[];
  enabled: boolean;
  is_favorite: boolean;
  created_at: string;
  updated_at: string;
}

export interface ModelWithProvider extends Model {
  provider_name: string;
  provider_type: DriverId;
  /**
   * Derived: model is configured and ready for use. Currently mirrors the
   * joined provider's `api_key_set && status === "active"`; not persisted.
   */
  healthy: boolean;
  /** Readonly profile with model capabilities (not persisted to database) */
  profile?: ModelProfile;
  /** Vendor/brand from the model registry; drives branding. Not persisted. */
  model_vendor?: ModelVendor;
}

// ============================================
// LLM Model Profile types
// Based on models.dev structure
// ============================================
/** Cost information for the model (per million tokens in USD) */
export interface ModelCost {
  /** Input cost per million tokens */
  input: number;
  /** Output cost per million tokens */
  output: number;
  /** Cached read cost per million tokens, if supported */
  cache_read?: number;
  /** Cache write cost per million tokens, if known */
  cache_write?: number;
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
  /** Cache write cost per million tokens, if known */
  cache_write?: number;
}

/** Token limits for the model */
export interface ModelLimits {
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
export interface ModelModalities {
  /** Supported input modalities */
  input: Modality[];
  /** Supported output modalities */
  output: Modality[];
}

/** Reasoning effort level for models that support it */
export type ReasoningEffort = "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";

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

export type Verbosity = "low" | "medium" | "high";

/** Named verbosity value for UI display */
export interface VerbosityValue {
  /** The API value (e.g., "low", "high") */
  value: Verbosity;
  /** Display name (e.g., "Low", "High") */
  name: string;
}

/** Verbosity configuration for a model */
export interface VerbosityConfig {
  /** Available verbosity values for this model */
  values: VerbosityValue[];
  /** Default verbosity for this model */
  default: Verbosity;
}

/**
 * LLM Model Profile describing model capabilities
 * Based on models.dev structure (https://models.dev/api.json)
 */
export interface ModelProfile {
  decisions?: DecisionModelProfile;
  /** Display name of the model */
  name: string;
  /** Model family (e.g., "gpt-5.6-sol", "claude-sonnet-5") */
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
  cost?: ModelCost;
  /** Token limits */
  limits?: ModelLimits;
  /** Supported modalities */
  modalities?: ModelModalities;
  /** Reasoning effort configuration (for reasoning models) */
  reasoning_effort?: ReasoningEffortConfig;
  /** Verbosity configuration (for models that support output-length control) */
  verbosity?: VerbosityConfig;
  /** Provider-advertised request parameters supported by this model */
  supported_parameters?: string[];
  /** Whether the model supports tool_search (deferred tool loading) */
  tool_search?: boolean;
  /** Whether the model supports native execution phases */
  supports_phases?: boolean;
}

export interface CreateModelRequest {
  model_id: string;
  service?: ModelService;
  profile_key?: string;
  display_name: string;
  capabilities?: string[];
  enabled?: boolean;
  is_favorite?: boolean;
}

export interface UpdateModelRequest {
  provider_id?: string;
  model_id?: string;
  service?: ModelService;
  profile_key?: string;
  display_name?: string;
  capabilities?: string[];
  enabled?: boolean;
  is_favorite?: boolean;
}
