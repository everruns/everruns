// App types (deployable agent+harness bundles with multi-channel support)

import type { PrincipalSummary } from "./common-types";

export type AppStatus = "draft" | "published" | "archived" | "deleted";
export type ChannelType = "slack" | "ag_ui" | "schedule" | "webhook" | "a2a";
export type SessionStrategy = "per_thread" | "per_channel" | "per_user";
export type SlackReplyMode = "all_messages" | "report_progress_only";
export type InvocationSessionMode = "shared_session" | "session_per_invocation";
export type AgUiToolVisibility = "none" | "generic" | "narrated";
export type AgentVersionPolicy = "default" | "latest" | "pinned";
export type AppEndpointAuthMode =
  | "anonymous"
  | "shared_secret"
  | "api_key"
  | "google_oidc"
  | "oidc"
  | "oauth2_introspection"
  | "http_basic"
  | "mtls";

export interface AppEndpointAuthRequirements {
  audiences?: string[];
  scopes?: string[];
  claims?: Record<string, unknown>;
  subjects?: string[];
  groups?: string[];
  domains?: string[];
}

export type AppEndpointAuthProviderConfig =
  | { type: "google_oidc"; client_id: string; allowed_domains?: string[] }
  | { type: "oidc"; issuer: string; jwks_url?: string }
  | {
      type: "oauth2_introspection";
      introspection_url: string;
      client_id?: string;
      client_secret?: string;
      client_secret_configured?: boolean;
    }
  | {
      type: "http_basic";
      username: string;
      password?: string;
      password_hash?: string;
      password_configured?: boolean;
    }
  | { type: "mtls"; header_name: string; allowed_values: string[] };

export interface AppEndpointAuthConfig {
  mode: AppEndpointAuthMode;
  provider?: AppEndpointAuthProviderConfig;
  requirements?: AppEndpointAuthRequirements;
}

export interface SlackChannelConfig {
  signing_secret?: string;
  signing_secret_configured?: boolean;
  bot_token?: string;
  bot_token_configured?: boolean;
  channel_id?: string;
  team_id?: string;
  session_strategy: SessionStrategy;
  reply_mode?: SlackReplyMode;
  webhook_verified_at?: string | null;
  first_message_received_at?: string | null;
}

/** Default thread expiration window for AG-UI (6 hours, in seconds). */
export const DEFAULT_AG_UI_SESSION_EXPIRATION_SECONDS = 6 * 60 * 60;
export const DEFAULT_AG_UI_GENERIC_TOOL_TEXT = "Working...";

export interface AgUiChannelConfig {
  anonymous?: boolean;
  /**
   * Optional shared token for the public AG-UI endpoint. When present, clients
   * must send it as `Authorization: Bearer <token>` or `X-Everruns-AG-UI-Token`.
   */
  token?: string;
  token_configured?: boolean;
  /**
   * How long an AG-UI thread can be resumed (in seconds) after the underlying
   * session was created. After this elapses the same `thread_id` cannot reuse
   * the existing session and must start a new one. `0` disables expiration.
   * Defaults to 6 hours.
   */
  session_expiration_seconds?: number;
  /**
   * Per-IP requests-per-minute cap on the public AG-UI endpoint for this
   * app. `0` or absent disables the per-app cap (the global API cap still
   * applies). Server-side validation rejects values above 1,000,000.
   */
  rate_limit_per_minute?: number;
  /**
   * Public tool activity visibility. Raw tool names, args, and results are
   * never exposed through the public AG-UI stream.
   */
  tool_visibility?: AgUiToolVisibility;
  /** Text shown while tools are running when tool_visibility is "generic". */
  generic_tool_text?: string;
  auth?: AppEndpointAuthConfig;
}

export interface ScheduleChannelConfig {
  cron_expression: string;
  timezone?: string;
  session_mode?: InvocationSessionMode;
  message: string;
}

export interface WebhookChannelConfig {
  token?: string;
  token_configured?: boolean;
  session_mode?: InvocationSessionMode;
  message: string;
}

/**
 * A2A (Agent2Agent) channel configuration.
 *
 * The plaintext API key is **never** returned by the API after creation /
 * regeneration. Reads only surface the non-secret display prefix; use
 * `addA2aChannel` / `regenerateA2aChannelKey` to obtain the plaintext (which is
 * shown exactly once).
 */
export interface A2aChannelConfig {
  api_key_hash?: string;
  api_key_prefix: string;
  session_mode?: InvocationSessionMode;
  message: string;
  agent_card_name?: string;
  agent_card_description?: string;
  /**
   * Optional per-IP rate limit applied to this app's A2A endpoint, in
   * requests per minute. `0` (or absent) disables the per-channel limit;
   * the global API limit still applies.
   */
  rate_limit_per_minute?: number;
  auth?: AppEndpointAuthConfig;
}

export interface AppChannel {
  id: string;
  channel_type: ChannelType;
  channel_config:
    | SlackChannelConfig
    | AgUiChannelConfig
    | ScheduleChannelConfig
    | WebhookChannelConfig
    | A2aChannelConfig
    | Record<string, unknown>;
  enabled: boolean;
  next_run_at?: string | null;
  last_invoked_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface AppRunEvent {
  id: string;
  app_id: string;
  channel_id: string;
  channel_type: ChannelType;
  channel_name?: string | null;
  status: "pending" | "running" | "completed" | "failed" | "skipped";
  created_at: string;
  completed_at?: string | null;
}

export interface AppRunBucket {
  hour: string;
  ok: number;
  err: number;
  running?: number;
}

export interface App {
  id: string;
  name: string;
  description: string | null;
  harness_id: string;
  agent_id: string | null;
  agent_version_policy: AgentVersionPolicy;
  agent_version_id: string | null;
  agent_identity_id?: string | null;
  owner_principal_id: string;
  resolved_owner_user_id?: string | null;
  owner?: PrincipalSummary | null;
  effective_owner?: PrincipalSummary | null;
  channels: AppChannel[];
  status: AppStatus;
  published_at: string | null;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
}

export interface CreateAppRequest {
  name: string;
  description?: string;
  harness_id: string;
  agent_id?: string;
  agent_version_policy?: AgentVersionPolicy;
  agent_version_id?: string;
  agent_identity_id?: string;
  channel_type?: ChannelType;
  channel_config?:
    | SlackChannelConfig
    | AgUiChannelConfig
    | ScheduleChannelConfig
    | WebhookChannelConfig
    | A2aChannelConfig
    | Record<string, unknown>;
}

export interface UpdateAppRequest {
  name?: string;
  description?: string;
  harness_id?: string;
  agent_id?: string;
  agent_version_policy?: AgentVersionPolicy;
  agent_version_id?: string | null;
  agent_identity_id?: string | null;
  status?: AppStatus;
}

export interface AddChannelRequest {
  channel_type: ChannelType;
  channel_config?:
    | SlackChannelConfig
    | AgUiChannelConfig
    | ScheduleChannelConfig
    | WebhookChannelConfig
    | A2aChannelConfig
    | Record<string, unknown>;
  enabled?: boolean;
}

export interface UpdateChannelRequest {
  channel_type?: ChannelType;
  channel_config?:
    | SlackChannelConfig
    | AgUiChannelConfig
    | ScheduleChannelConfig
    | WebhookChannelConfig
    | A2aChannelConfig
    | Record<string, unknown>;
  enabled?: boolean;
}
