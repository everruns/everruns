import type { DriverId } from "./provider-driver-types";

export type ProviderStatus = "active" | "disabled";

/**
 * Configuration for linking from the chat UI to a provider's observability
 * dashboard ("trace"/"logs"). URL templates support the `{response_id}`,
 * `{session_id}`, `{turn_id}` and `{model}` placeholders. The backend resolves
 * driver defaults overlaid with the org's stored overrides; `enabled` gates
 * whether links are shown at all.
 */
export interface ProviderTraceConfig {
  enabled: boolean;
  generation_url_template?: string;
  session_url_template?: string;
}

/** One extra HTTP header sent with every request to a provider connection. */
export interface ProviderRequestHeader {
  name: string;
  value: string;
}

/** Per-connection extra request headers and prompt-cache diagnostics opt-in (Anthropic today). */
export interface ProviderRequestOptions {
  headers?: ProviderRequestHeader[];
  cache_diagnostics?: boolean;
}

export interface Provider {
  /** Connection-level chat API; "default" restores the driver default. */
  api?: string;
  id: string;
  name: string;
  provider_type: DriverId;
  base_url?: string;
  api_key_set: boolean;
  status: ProviderStatus;
  /** Whether this provider is host-managed (read-only to org admins). */
  managed: boolean;
  /** Most recent successful model sync from the provider's API. */
  last_synced_at?: string;
  /** When someone last reviewed this provider's discovered models. */
  models_reviewed_at?: string;
  created_at: string;
  updated_at: string;
  /** Resolved trace/observability link configuration (driver defaults + overrides). */
  trace?: ProviderTraceConfig;
  /** Extra headers and diagnostics options applied to every request to this provider. */
  request_options?: ProviderRequestOptions;
}

export interface CreateProviderRequest {
  /** Connection-level chat API; "default" restores the driver default. */
  api?: string;
  name: string;
  provider_type: DriverId;
  base_url?: string;
  api_key?: string;
  /** Typed credential fields keyed by the driver's credential-schema field names. */
  credentials?: Record<string, string>;
  trace?: ProviderTraceConfig;
  request_options?: ProviderRequestOptions;
}

export interface CheckCredentialsRequest {
  provider_type: DriverId;
  api_key?: string;
  /** Typed credential fields keyed by the driver's credential-schema field names. */
  credentials?: Record<string, string>;
  base_url?: string;
}

/**
 * Outcome of probing a candidate API key against the provider. Only
 * `rejected` proves the key is bad: `unsupported` (driver has no check) and
 * `unreachable` (outage/network) must not block the user.
 */
export type CredentialCheckResult =
  | { status: "valid"; models: number }
  | { status: "rejected"; message: string }
  | { status: "unsupported" }
  | { status: "unreachable"; message: string };

export interface UpdateProviderRequest {
  /** Connection-level chat API; "default" restores the driver default. */
  api?: string;
  name?: string;
  provider_type?: DriverId;
  base_url?: string;
  api_key?: string;
  /** Typed credential fields keyed by the driver's credential-schema field names. */
  credentials?: Record<string, string>;
  status?: ProviderStatus;
  trace?: ProviderTraceConfig;
  request_options?: ProviderRequestOptions;
}

/** A single declared credential field (mirrors core `FormField`). */
export interface CredentialFormField {
  name: string;
  label: string;
  field_type: "password" | "text" | "url";
  required: boolean;
  placeholder?: string;
  help_text?: string;
  /** Default value the form pre-fills (e.g. an OAuth scope or AWS region). */
  default_value?: string;
  /** Mutually-exclusive group label; fields sharing it are one credential method. */
  group?: string;
}

/** A driver's declared credential schema (mirrors core `CredentialFormSchema`). */
export interface CredentialFormSchema {
  fields: CredentialFormField[];
  instructions_markdown: string;
}

/** Per-driver credential schema from `GET /v1/providers/config`. */
export interface DriverCredentialInfo {
  /** Explicit chat API choices advertised by the server. */
  apis?: string[];
  services?: import("./model-types").ModelService[];
  driver: DriverId;
  credential_schema: CredentialFormSchema;
  supports_oauth: boolean;
}

/** Response shape of `GET /v1/providers/config`. */
export interface ProvidersConfigResponse {
  policies: Record<string, boolean>;
  drivers: DriverCredentialInfo[];
}
