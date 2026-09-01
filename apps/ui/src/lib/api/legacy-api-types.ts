// From legacy agent-identity-types.ts; retained as UI compatibility over generated OpenAPI schemas.

// Enums that stay generated (closed sets the server owns) while the entity they
// annotate is still hand-maintained here.
import type { LlmRetryInfo, SessionActivity, SessionSource } from "./schema-types";

// Agent Identity types
export type AgentIdentityStatus = "active" | "archived" | "deleted";

export interface AgentIdentity {
  id: string;
  name: string;
  description?: string | null;
  avatar_url?: string | null;
  locale?: string | null;
  timezone?: string | null;
  principal?: PrincipalSummary | null;
  effective_owner?: PrincipalSummary | null;
  status: AgentIdentityStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export interface CreateAgentIdentityRequest {
  name: string;
  description?: string;
  avatar_url?: string;
  locale?: string;
  timezone?: string;
}

export interface UpdateAgentIdentityRequest {
  name?: string;
  description?: string | null;
  avatar_url?: string | null;
  locale?: string | null;
  timezone?: string | null;
  status?: AgentIdentityStatus;
}

// From legacy agent-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// ============================================
// Agent types (M2)
// ============================================
export type AgentStatus = "active" | "archived" | "deleted";

export interface AgentHarnessSummary {
  id: string | null;
  name: string | null;
  display_name: string | null;
  source: "explicit" | "organization_default";
  status: "active" | "archived" | "deleted" | "unresolved";
}

export interface Agent {
  id: string;
  /** Addressable name (slug): lowercase alphanumeric and hyphens (e.g. "customer-support") */
  name: string;
  /** Human-readable display name shown in UI. Falls back to name when absent. */
  display_name: string | null;
  description: string | null;
  system_prompt: string;
  /** Base execution harness this agent runs on. Required; defaults to the org's built-in `generic` harness. */
  harness_id: string;
  default_model_id: string | null;
  default_version_id?: string | null;
  forked_from_agent_id?: string | null;
  forked_from_version_id?: string | null;
  root_agent_id?: string | null;
  tags: string[];
  /** Capabilities with per-agent configuration */
  capabilities: AgentCapabilityConfig[];
  /** Initial files. Optional: older records and serializers that strip empty arrays may omit this field. */
  initial_files?: InitialFile[];
  /** Tool definitions (including client-side tools), defaults to [] */
  tools?: ToolDefinition[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  status: AgentStatus;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
  /** Cumulative token usage across all sessions for this agent */
  usage?: TokenUsage;
  /** Number of sessions using this agent. Present on list/detail API responses. */
  session_count?: number;
  /** Number of non-deleted apps using this agent. Present on list/detail API responses. */
  app_count?: number;
  /** Harness a newly created session for this agent will resolve to. */
  effective_harness?: AgentHarnessSummary;
}

export type AgentVersionChangeKind =
  | "auto"
  | "manual"
  | "patch"
  | "minor"
  | "major"
  | "import"
  | "rollback"
  | "fork";

export interface AgentVersion {
  id: string;
  agent_id: string;
  version_number: number;
  semver_major: number;
  semver_minor: number;
  semver_patch: number;
  version: string;
  is_published: boolean;
  parent_version_id: string | null;
  source_version_id: string | null;
  created_by_principal_id: string | null;
  change_kind: AgentVersionChangeKind;
  summary: string | null;
  config_hash: string;
  authored_config: Record<string, unknown>;
  resolved_config: Record<string, unknown>;
  created_at: string;
}

export interface CreateAgentVersionRequest {
  summary?: string;
  change_kind?: AgentVersionChangeKind;
}

export interface SetDefaultAgentVersionRequest {
  version_id: string;
}

export interface RollbackAgentVersionRequest {
  save_version: boolean;
  summary?: string;
}

export interface ForkAgentVersionRequest {
  name: string;
  display_name?: string;
  description?: string;
}

export interface AgentVersionDiffResponse {
  from_version_id: string;
  to_version_id: string;
  authored_diff: Record<
    string,
    {
      from: unknown;
      to: unknown;
    }
  >;
  resolved_diff: Record<
    string,
    {
      from: unknown;
      to: unknown;
    }
  >;
}

export interface CreateAgentRequest {
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
  description?: string;
  system_prompt: string;
  /** Base execution harness (id). Mutually exclusive with `harness_name`. Omit both to default to the org's built-in `generic` harness. */
  harness_id?: string;
  /** Base execution harness (name), resolved within the org. Mutually exclusive with `harness_id`. */
  harness_name?: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  initial_files?: InitialFile[];
  /** Tool definitions (including client-side tools) */
  tools?: ToolDefinition[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList;
}

export interface UpdateAgentRequest {
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name?: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
  description?: string;
  system_prompt?: string;
  /** Base execution harness (id). Omit to leave unchanged; never clearable to null. Mutually exclusive with `harness_name`. */
  harness_id?: string;
  /** Base execution harness (name), resolved within the org. Mutually exclusive with `harness_id`. */
  harness_name?: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  initial_files?: InitialFile[];
  status?: AgentStatus;
  /** Tool definitions (including client-side tools) */
  tools?: ToolDefinition[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  /**
   * Session this one was forked from (knowledge/runtime-resources/forking-sessions.md).
   * NULL for sessions that were not forked. Distinct from a subagent's parent.
   */
  forked_from_session_id?: string | null;
  /** Parent event sequence the fork was taken at. NULL unless this is a fork. */
  forked_from_sequence?: number | null;
}

/** Read-only agent example defined in code, adoptable as a real Agent */
export interface AgentExample {
  name: string;
  display_name: string;
  description: string;
  tags: string[];
  capabilities: AgentCapabilityConfig[];
  dev_only: boolean;
}

/** Request to preview the final agent shape with capabilities applied */
export interface PreviewAgentRequest {
  /** The base system prompt (before capability additions) */
  system_prompt: string;
  /** Capabilities to apply with per-agent configuration */
  capabilities?: AgentCapabilityConfig[];
  /** Client-side tools to include in preview output */
  tools?: ToolDefinition[];
}

export type FindingSeverity = "warning" | "info" | "suggestion";

export type FindingCategory = "structure" | "completeness" | "effectiveness" | "safety" | "cost";

export type FindingSource = "builtin" | "llm" | "health_check";

/** Pointer to the config field (and optional byte span) a finding refers to */
export interface FindingLocation {
  field: string;
  start?: number;
  end?: number;
}

/** Advisory finding from agent config checks (knowledge/evaluation/agent-checks.md) */
export interface AgentFinding {
  /** Stable rule identifier, e.g. "prompt.duplicate_paragraphs" */
  rule_id: string;
  severity: FindingSeverity;
  category: FindingCategory;
  message: string;
  location?: FindingLocation;
  /** Proposed replacement text (phase 2+) */
  fix?: string;
  source: FindingSource;
}

/** Response from on-demand agent analysis (built-in rules + LLM checkers) */
export interface AgentAnalysisResponse {
  findings: AgentFinding[];
}

export type HealthCheckStatus = "pending" | "running" | "completed" | "failed";

/** Outcome of a single generated health-check case */
export interface HealthCheckCaseResult {
  name: string;
  user_message: string;
  rubric: string;
  /** Public ID of the real session created for this case */
  session_id?: string;
  passed: boolean;
  score: number;
  judge_reason: string;
  deterministic_reason: string;
  turns: number;
  latency_ms: number;
  /** Input tokens for this case (agent turns + judge). */
  input_tokens?: number;
  /** Output tokens for this case (agent turns + judge). */
  output_tokens?: number;
  error?: string;
}

/** Aggregate metrics across all cases in a health-check run */
export interface HealthCheckSummary {
  total: number;
  passed: number;
  failed: number;
  errored: number;
  pass_rate: number;
  avg_score: number;
  avg_turns: number;
  total_input_tokens: number;
  total_output_tokens: number;
}

/** A behavioral health-check run (knowledge/evaluation/agent-checks.md, tier-3) */
export interface HealthCheckRun {
  id: string;
  agent_id?: string;
  config_hash: string;
  model_id?: string;
  status: HealthCheckStatus;
  summary?: HealthCheckSummary;
  results?: HealthCheckCaseResult[];
  error_message?: string;
  created_at: string;
  completed_at?: string;
}

/** Latest health-check run for an agent plus a stale-config flag (EVE-588) */
export interface LatestHealthCheckRun {
  /** The latest run, or absent if the agent has never been health-checked */
  run?: HealthCheckRun;
  /** True when the run was executed against a different config than current */
  config_changed: boolean;
}

/** Response showing the final agent shape after applying capabilities */
export interface AgentPreviewResponse {
  /** The full system prompt with capability additions prepended */
  system_prompt: string;
  /** All tool definitions from capabilities */
  tools: ToolDefinition[];
  /** Advisory findings from built-in checks (absent on harness preview) */
  findings?: AgentFinding[];
}

// ============================================
// Harness types
// ============================================
export type HarnessStatus = "active" | "archived" | "deleted";

export interface Harness {
  id: string;
  /** Addressable name (slug): lowercase alphanumeric and hyphens (e.g. "my-harness") */
  name: string;
  /** Human-readable display name shown in UI. Falls back to name when absent. */
  display_name: string | null;
  description: string | null;
  /** Base system prompt. Null/absent means the harness contributes no base prompt. */
  system_prompt?: string | null;
  parent_harness_id: string | null;
  default_model_id: string | null;
  tags: string[];
  /** Capabilities with per-harness configuration */
  capabilities: AgentCapabilityConfig[];
  /** Initial files. Optional: older records and serializers that strip empty arrays may omit this field. */
  initial_files?: InitialFile[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  /** Whether this harness is built-in (system-managed, readonly) */
  is_built_in: boolean;
  status: HarnessStatus;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
  /** Number of sessions using this harness. Present on list/detail API responses. */
  session_count?: number;
  /** Number of non-deleted apps using this harness. Present on list/detail API responses. */
  app_count?: number;
}

export interface CreateHarnessRequest {
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
  description?: string;
  /** Optional base system prompt. Omit for a harness with no base prompt. */
  system_prompt?: string;
  parent_harness_id?: string;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-harness configuration */
  capabilities?: AgentCapabilityConfig[];
  initial_files?: InitialFile[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList;
}

export interface UpdateHarnessRequest {
  /** Addressable name (slug): lowercase alphanumeric and hyphens */
  name?: string;
  /** Human-readable display name shown in UI */
  display_name?: string;
  description?: string;
  system_prompt?: string;
  parent_harness_id?: string | null;
  default_model_id?: string;
  tags?: string[];
  /** Capabilities with per-harness configuration */
  capabilities?: AgentCapabilityConfig[];
  initial_files?: InitialFile[];
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  status?: HarnessStatus;
}

/** Read-only harness example defined in code, adoptable as a real Harness */
export interface HarnessExample {
  name: string;
  display_name: string;
  description: string;
  tags: string[];
  /** Name of the parent harness (e.g. `generic`) the example will inherit from when imported. */
  parent_name?: string;
  capabilities: AgentCapabilityConfig[];
  dev_only: boolean;
}

/** Request to preview the final harness shape with capabilities applied */
export interface PreviewHarnessRequest {
  /** Optional base system prompt (before capability additions) */
  system_prompt?: string;
  /** Optional parent harness to inherit from before previewing local changes */
  parent_harness_id?: string;
  /** Capability IDs to apply */
  capabilities?: AgentCapabilityConfig[];
}

// From legacy app-types.ts; retained as UI compatibility over generated OpenAPI schemas.
export type AppStatus = "draft" | "published" | "archived" | "deleted";

export type ChannelType =
  | "slack"
  | "ag_ui"
  | "schedule"
  | "webhook"
  | "a2a"
  | "fcp"
  | "public_chat";

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
  | {
      type: "google_oidc";
      client_id: string;
      allowed_domains?: string[];
    }
  | {
      type: "oidc";
      issuer: string;
      jwks_url?: string;
    }
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
  | {
      type: "mtls";
      header_name: string;
      allowed_values: string[];
      proxy_secret_header?: string;
      /** Write-only. Never returned after save; presence indicated by proxy_secret_configured. */
      proxy_secret?: string;
      proxy_secret_configured?: boolean;
    };

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
  reasoning_summary_visible?: boolean;
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
 * FCP (Free Communication Protocol) channel configuration.
 *
 * Minimal text-first ingress. FCP intentionally runs its own auth stack —
 * anonymous + an optional shared bearer token — and does not accept the
 * inline `AppEndpointAuthConfig` modes used by AG-UI and A2A.
 */
export interface FcpChannelConfig {
  anonymous?: boolean;
  /**
   * Optional shared bearer token. When set, clients must send
   * `Authorization: Bearer <token>` or `X-Everruns-FCP-Token: <token>`.
   */
  token?: string;
  token_configured?: boolean;
  /** Markdown body returned on `GET` (the FCP handshake). */
  handshake?: string;
  /** Seconds the `fcp_session` cookie keeps a session resumable. 0 disables. */
  session_expiration_seconds?: number;
  /** Per-IP requests-per-minute cap. `0` or absent disables the per-app cap. */
  rate_limit_per_minute?: number;
  /** Max seconds to wait for the agent's reply before returning 504. */
  response_timeout_seconds?: number;
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
  /**
   * Optional shared HMAC signing secret. Plaintext is **write-only** and
   * only sent on a PATCH that intends to rotate / set the secret;
   * subsequent reads expose only the `signing_secret_configured` flag.
   * When set, A2A requests must additionally carry timestamp + signature
   * headers (TM-A2A-010).
   */
  signing_secret?: string;
  /** Read-only flag indicating whether a signing secret is configured. */
  signing_secret_configured?: boolean;
}

/** Branding shown on a Public Chat surface. All fields optional. */
export interface PublicChatBranding {
  display_name?: string;
  logo_url?: string;
  primary_color?: string;
  welcome_message?: string;
}

/**
 * CAPTCHA / bot-mitigation config for a Public Chat channel (Cloudflare
 * Turnstile). `secret_key` is write-only; reads expose only
 * `secret_key_configured`. `site_key` is public.
 */
export interface PublicChatCaptchaConfig {
  provider?: "turnstile";
  enabled?: boolean;
  site_key: string;
  secret_key?: string;
  secret_key_configured?: boolean;
}

/**
 * Public Chat channel configuration. Reuses AG-UI streaming semantics plus
 * branding and bot-mitigation tailored to a public, link-shareable chat.
 */
export interface PublicChatChannelConfig {
  anonymous?: boolean;
  token?: string;
  token_configured?: boolean;
  session_expiration_seconds?: number;
  rate_limit_per_minute?: number;
  tool_visibility?: AgUiToolVisibility;
  generic_tool_text?: string;
  auth?: AppEndpointAuthConfig;
  branding?: PublicChatBranding;
  captcha?: PublicChatCaptchaConfig;
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
    | FcpChannelConfig
    | PublicChatChannelConfig
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
    | FcpChannelConfig
    | PublicChatChannelConfig
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
    | FcpChannelConfig
    | PublicChatChannelConfig
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
    | FcpChannelConfig
    | PublicChatChannelConfig
    | Record<string, unknown>;
  enabled?: boolean;
}

// From legacy auth-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Authentication, user, and organization types
// ============================================
// Authentication types
// ============================================
export type AuthMode = "none" | "admin" | "full" | "external";

export interface FeatureFlags {
  notifications: boolean;
  evals: boolean;
  /** Skills registry management UI. Experimental. */
  skills: boolean;
  /** Workspace memory management UI. Experimental. */
  memory: boolean;
  /** Knowledge index management UI. Experimental. */
  knowledge: boolean;
  /** Plugin marketplace and installed-plugin management UI. Experimental. */
  plugins: boolean;
  app_budgets: boolean;
  agent_versions: boolean;
  voice: boolean;
  /** Outbound agent delegation (`a2a_agent_delegation`, `agent_handoff`). Experimental. */
  agent_delegation: boolean;
  /** Observers: online scoring of production sessions. Experimental. */
  observers: boolean;
  /** Public Chat (isolated public-facing chat web app + `public_chat` channel). Experimental. */
  public_chat: boolean;
  /** Browser-native tools exposed by the authenticated Everruns UI. Experimental. */
  webmcp: boolean;
  /** Machine-payment custody, policy, audit, and paid capability surfaces. */
  machine_payments: boolean;
}

export interface OrgFeatureFlagSetting {
  name: string;
  label: string;
  description: string;
  experimental: boolean;
  system_enabled: boolean;
  org_enabled: boolean;
  effective: boolean;
}

export interface OrgFeatureFlagsSettingsResponse {
  flags: OrgFeatureFlagSetting[];
}

export interface AuthConfigResponse {
  mode: AuthMode;
  /** Trusted configured origin hosting the login page. */
  login_origin?: string;
  password_auth_enabled: boolean;
  oauth_providers: string[];
  signup_enabled: boolean;
  /** True when email signup ends at a "check your email" confirmation. */
  signup_email_confirm?: boolean;
  /** Bot-mitigation challenge for abuse-prone auth forms, when configured. */
  captcha?: AuthCaptchaConfig;
}

export interface AuthCaptchaConfig {
  provider: string;
  site_key: string;
}

/** Response from GET /v1/{resource}/config */
export interface ResourceConfigResponse {
  policies: Record<string, boolean>;
}

export interface LoginRequest {
  email: string;
  password: string;
}

export interface RegisterRequest {
  email: string;
  password: string;
  /** Captcha token; required when /v1/auth/config advertises a captcha. */
  captcha_token?: string;
}

export interface TokenResponse {
  access_token: string;
  token_type: string;
  expires_in: number;
  refresh_token?: string;
}

/** Request to begin a password reset (enumeration-safe). */
export interface ForgotPasswordRequest {
  email: string;
  /** Captcha token; required when /v1/auth/config advertises a captcha. */
  captcha_token?: string;
}

/** Request to complete a password reset using an emailed token. */
export interface ResetPasswordRequest {
  token: string;
  password: string;
}

/** Request to verify an email address using an emailed token. */
export interface VerifyEmailRequest {
  token: string;
}

/** Request to re-send a verification email (enumeration-safe). */
export interface ResendVerificationRequest {
  email: string;
}

/** Generic success response returned by recovery/verification endpoints. */
export interface OkResponse {
  ok: boolean;
}

/** Organization role */
export type OrgRole = "owner" | "admin" | "member";

/** Organization membership info */
export interface OrganizationMembership {
  public_id: string;
  name: string;
  role: OrgRole;
}

/** Full organization details (from API) */
export interface Organization {
  id: string;
  name: string;
  default_model_id: string | null;
  default_harness_id: string | null;
  base_harness_id: string | null;
  /**
   * Org-level default provider per service (EVE-569): service kind -> provider id.
   * Always present in responses (empty object when no defaults are configured).
   */
  default_provider_per_service: Record<string, string>;
  created_at: string;
  updated_at: string;
  /**
   * When the org's creator finished or skipped the setup wizard. `null` means
   * onboarding is incomplete; the resume redirect sends the user to /setup.
   */
  onboarding_completed_at: string | null;
}

/** Request to create an organization */
export interface CreateOrganizationRequest {
  name: string;
}

/** Request to update an organization */
export interface UpdateOrganizationRequest {
  name?: string;
  default_model_id?: string | null;
  default_harness_id?: string;
  base_harness_id?: string;
  /**
   * Org-level default provider per service (EVE-569): service kind -> provider id.
   * When present it replaces the whole map.
   */
  default_provider_per_service?: Record<string, string>;
}

export interface UserInfoResponse {
  id: string;
  email: string;
  name: string;
  roles: string[];
  avatar_url?: string;
  /** Whether the account's email has been verified. Drives the in-app verify nudge. */
  email_verified: boolean;
  /** Organizations the user belongs to */
  organizations?: OrganizationMembership[];
}

export interface PersonalAccessTokenResponse {
  id: string;
  name: string;
  token: string;
  token_prefix: string;
  scopes: string[];
  expires_at?: string;
  created_at: string;
}

export interface PersonalAccessTokenListItem {
  id: string;
  name: string;
  token_prefix: string;
  scopes: string[];
  expires_at?: string;
  last_used_at?: string;
  created_at: string;
}

export interface CreatePersonalAccessTokenRequest {
  name: string;
  scopes?: string[];
  expires_in_days?: number;
}

export interface RefreshTokenRequest {
  refresh_token: string;
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

/** Request to update current user profile */
export interface UpdateProfileRequest {
  name: string;
}

/** Response from profile update */
export interface ProfileResponse {
  id: string;
  email: string;
  name: string;
  avatar_url?: string;
}

// ============================================
// User Connection types
// ============================================
export interface UserConnection {
  provider: string;
  connection_type: string;
  provider_username?: string;
  scopes?: string;
  connected_at: string;
}

export interface ConnectionProvider {
  provider_id: string;
  display_name: string;
  description: string;
  icon: string;
  connection_type: "oauth" | "api_key";
  form_schema?: ConnectionFormSchema;
}

export interface ConnectionFormSchema {
  fields: ConnectionFormField[];
  instructions_markdown: string;
}

export interface ConnectionFormField {
  name: string;
  label: string;
  field_type: "password" | "text" | "url";
  required: boolean;
  placeholder?: string;
  help_text?: string;
}

export interface VerifyConnectionResponse {
  valid: boolean;
  error?: string;
}

// From legacy budget-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Budget API types — mirrors `crates/core/src/budget.rs`.
// Behind the `app_budgets` feature flag for app/channel subjects.
export type BudgetSubjectType = "session" | "agent" | "user" | "org" | "app" | "app_channel";

export type BudgetStatus = "active" | "paused" | "exhausted" | "disabled";

/**
 * Period configuration for recurring budgets. Drives automatic balance reset.
 *  - `Duration` is a sliding window of `seconds` from `period_started_at`.
 *  - `Rolling` accepts shorthand like `5h`, `24h`, `30d` (server normalises).
 *  - `Calendar` aligns to UTC `hour | day | week | month | year` boundaries.
 */
export type BudgetPeriod =
  | {
      type: "duration";
      seconds: number;
    }
  | {
      type: "rolling";
      window: string;
    }
  | {
      type: "calendar";
      unit: string;
    };

export interface Budget {
  id: string;
  organization_id: string;
  subject_type: BudgetSubjectType;
  subject_id: string;
  currency: string;
  limit: number;
  soft_limit?: number | null;
  balance: number;
  period?: BudgetPeriod | null;
  period_started_at?: string | null;
  metadata?: Record<string, unknown> | null;
  status: BudgetStatus;
  created_at: string;
  updated_at: string;
}

export interface CreateBudgetRequest {
  subject_type: BudgetSubjectType;
  subject_id: string;
  currency: string;
  limit: number;
  soft_limit?: number | null;
  period?: BudgetPeriod | null;
  metadata?: Record<string, unknown> | null;
}

export interface UpdateBudgetRequest {
  limit?: number;
  soft_limit?: number | null;
  status?: BudgetStatus;
  metadata?: Record<string, unknown> | null;
}

// From legacy capability-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// NOTE: CapabilityId is defined in common-types for proper ordering
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
  /** Whether this capability is a guardrail (constrains agent behavior) */
  is_guardrail?: boolean;
  /** IDs of capabilities that this capability depends on */
  dependencies?: CapabilityId[];
  /** UI feature strings this capability contributes to */
  features?: string[];
  /** JSON Schema for capability-specific config */
  config_schema?: Record<string, unknown>;
  /** react-jsonschema-form uiSchema hints for rendering config_schema */
  config_ui_schema?: Record<string, unknown>;
  /** Number of active agents in the org referencing this capability */
  agent_count?: number;
  /** Number of active harnesses in the org referencing this capability */
  harness_count?: number;
  /** Slug under https://dev.everruns.com/capabilities/ for the public docs page */
  docs_slug?: string;
  /**
   * Localized display strings keyed by lowercase language tag (e.g. "uk").
   * The "en" entry carries only config_description, since the base
   * name/description/config_schema strings are already English.
   */
  localizations?: Record<string, CapabilityLocalization>;
}

/** Localized display strings for one locale of a capability. */
export interface CapabilityLocalization {
  name?: string;
  description?: string;
  /** One-line summary of what this capability's config controls */
  config_description?: string;
  /**
   * Overlay merged into config_schema before rendering: mirrors the schema
   * structure (properties/items) with title/description/enum_labels leaves.
   */
  config_overlay?: Record<string, unknown>;
}

export interface DeclarativeCapabilityFile {
  path: string;
  content: string;
  access?: "readonly" | "readwrite";
}

export interface DeclarativeCapabilitySkillFile {
  path: string;
  content: string;
}

export interface DeclarativeCapabilitySkill {
  name: string;
  description: string;
  instructions: string;
  files?: DeclarativeCapabilitySkillFile[];
  user_invocable?: boolean;
  disable_model_invocation?: boolean;
}

export interface DeclarativeCapabilityDefinition {
  name: string;
  display_name?: string | null;
  description: string;
  icon?: string;
  category?: string;
  system_prompt?: string;
  mcp_servers?: Record<string, unknown>;
  skills?: DeclarativeCapabilitySkill[];
  files?: DeclarativeCapabilityFile[];
  dependencies?: CapabilityId[];
  features?: string[];
  risk_level?: "low" | "medium" | "high";
}

export interface DeclarativeCapability {
  id: string;
  capability_id: CapabilityId;
  name: string;
  display_name?: string | null;
  description: string;
  status: "active" | "disabled" | "archived" | "deleted";
  definition: DeclarativeCapabilityDefinition;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export interface CreateDeclarativeCapabilityRequest {
  definition: DeclarativeCapabilityDefinition;
}

export interface UpdateDeclarativeCapabilityRequest {
  definition?: DeclarativeCapabilityDefinition;
  status?: "active" | "disabled" | "archived";
}

// From legacy command-types.ts; retained as UI compatibility over generated OpenAPI schemas.
export type CommandSource = "system" | "skill";

export interface CommandArg {
  name: string;
  description: string;
  required: boolean;
}

export interface CommandDescriptor {
  name: string;
  description: string;
  source: CommandSource;
  args?: CommandArg[];
}

export interface CommandsResponse {
  commands: CommandDescriptor[];
}

export interface ExecuteCommandRequest {
  name: string;
  arguments?: string;
  controls?: Controls;
}

export interface CommandResult {
  success: boolean;
  message: string;
  error_code?: string;
  error_fields?: Record<string, unknown>;
}

// From legacy common-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Common/shared types used across multiple domains
// ============================================
// Capability ID and config (used by agents, harnesses, capabilities)
// ============================================
/** Capability ID - extensible string-based identifier */
export type CapabilityId = string;

/** Per-agent capability configuration */
export interface AgentCapabilityConfig {
  /** Reference to the capability ID */
  ref: CapabilityId;
  /** Per-agent configuration for this capability (capability-specific) */
  config: Record<string, unknown>;
}

export interface InitialFile {
  path: string;
  content: string;
  encoding: "text" | "base64";
  is_readonly: boolean;
}

export type PrincipalKind = "user" | "agent_identity" | "system";

export interface PrincipalSummary {
  id: string;
  kind: PrincipalKind;
  subject_id?: string | null;
  metadata: Record<string, unknown>;
}

// ============================================
// Network access list (used by agents, harnesses, sessions)
// ============================================
/**
 * Controls which hosts/URLs an agent session can reach.
 * Merged across layers: allowed=intersect, blocked=union.
 */
export interface NetworkAccessList {
  /** Allowed host patterns (e.g. "example.com", "*.github.com", "https://api.example.com/v1/") */
  allowed?: string[];
  /** Blocked host patterns. Always denied, even if matched by allowed. */
  blocked?: string[];
}

// ============================================
// Tool types
// ============================================
export type ToolPolicy = "auto" | "requires_approval";

/**
 * Semantic hints describing a tool's behavioral properties.
 * All fields are optional — undefined means "unspecified".
 * Follows MCP tool annotations convention plus everruns-specific hints.
 */
export interface ToolHints {
  /** Tool does not modify any state (read-only queries, lookups) */
  readonly?: boolean;
  /** Tool may irreversibly destroy or delete data */
  destructive?: boolean;
  /** Same args produce same effect; safe to retry */
  idempotent?: boolean;
  /** Interacts with external entities (network, APIs, cloud services) */
  open_world?: boolean;
  /** Needs API keys or credentials to function */
  requires_secrets?: boolean;
  /** May take significant time to complete (> ~5s typical) */
  long_running?: boolean;
}

/** Tool definition - builtin tool configuration */
export interface BuiltinTool {
  type: "builtin";
  /** Tool name (used by LLM and for registry lookup) */
  name: string;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  /** Tool description for LLM */
  description: string;
  /** JSON schema for tool parameters */
  parameters: Record<string, unknown>;
  /** Tool policy (auto or requires_approval) */
  policy?: ToolPolicy;
  /** Semantic hints describing the tool's behavioral properties */
  hints?: ToolHints;
}

/** Tool definition - client-side tool executed by the caller */
export interface ClientSideTool {
  type: "client_side";
  /** Tool name (used by LLM and for registry lookup) */
  name: string;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  /** Tool description for LLM */
  description: string;
  /** JSON schema for tool parameters */
  parameters: Record<string, unknown>;
  /** Semantic hints describing the tool's behavioral properties */
  hints?: ToolHints;
}

/** Tool definition - builtin or client-side */
export type ToolDefinition = BuiltinTool | ClientSideTool;

// ============================================
// Token usage
// ============================================
/** Token usage statistics */
export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  /** Tokens read from cache (reduces cost) */
  cache_read_tokens?: number;
  /** Tokens written to cache (Anthropic-specific) */
  cache_creation_tokens?: number;
  /**
   * Actual cost of this generation in USD, as reported by the provider inline
   * (e.g. OpenRouter's `usage.cost`). Absent for providers that do not report one.
   */
  actual_cost_usd?: number;
  /**
   * Estimated cost of this generation in USD, derived from the model's price-table
   * profile. Absent when there is no profile cost data for the model. Tracked
   * independently of `actual_cost_usd` so estimate-vs-actual drift can be reconciled.
   */
  estimated_cost_usd?: number;
  /**
   * Best-effort cost in USD. Aggregate usage uses this to preserve the stored
   * actual-else-estimated total across mixed generations.
   */
  effective_cost_usd?: number;
}

export interface ContextReportSection {
  key: string;
  label: string;
  tokens: number;
  items: number;
}

export interface ContextReportContribution {
  section_key: string;
  source_id: string;
  label: string;
  tokens: number;
}

export interface SessionContextReport {
  session_id: string;
  model: string;
  context_window_tokens?: number;
  estimated_input_tokens: number;
  sections: ContextReportSection[];
  contributions: ContextReportContribution[];
  cumulative_usage?: TokenUsage;
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
// Health check
// ============================================
export interface HealthResponse {
  status: string;
  version: string;
  runner_mode: string;
}

// From legacy durable-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Durable Execution types
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
  workflow_id?: string;
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
  workflow_id?: string;
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
  completed_tasks: number;
  failed_tasks: number;
  started_tasks: number;
  queue_depth_by_type?: Record<string, number>;
  // Workflows
  running_workflows: number;
  pending_workflows: number;
  completed_workflows: number;
  failed_workflows: number;
  started_workflows: number;
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

/** Single metrics data point (timestamped health snapshot) */
export interface MetricsPoint {
  timestamp: string;
  running_workflows: number;
  pending_workflows: number;
  pending_tasks: number;
  claimed_tasks: number;
  active_workers: number;
  load_percentage: number;
  dlq_size: number;
  tasks_completed_total: number;
  tasks_failed_total: number;
  tasks_started_total: number;
  workflows_completed_total: number;
  workflows_failed_total: number;
  workflows_started_total: number;
}

/** Metrics time series response */
export interface MetricsTimeSeriesResponse {
  points: MetricsPoint[];
  resolution_seconds: number;
}

// ============================================
// Durable Schedule types
// ============================================
/** Schedule target type */
export type ScheduleTargetType = "workflow" | "activity";

/** Schedule execution status */
export type ScheduleExecutionStatus = "pending" | "running" | "completed" | "failed" | "skipped";

/** Schedule target configuration */
export interface ScheduleTarget {
  type: ScheduleTargetType;
  name: string;
  input: Record<string, unknown>;
}

/** Scheduled task */
export interface DurableSchedule {
  id: string;
  name: string;
  description?: string;
  cron_expression: string;
  timezone: string;
  target: ScheduleTarget;
  enabled: boolean;
  max_concurrent?: number;
  catch_up_missed: boolean;
  max_catch_up?: number;
  retry_policy?: Record<string, unknown>;
  last_triggered_at?: string;
  next_trigger_at?: string;
  created_at: string;
  updated_at: string;
}

/** Schedule execution record */
export interface ScheduleExecution {
  id: string;
  schedule_id: string;
  scheduled_at: string;
  started_at: string;
  completed_at?: string;
  status: ScheduleExecutionStatus;
  workflow_id?: string;
  task_id?: string;
  error?: string;
  duration_ms?: number;
  created_at: string;
}

/** Schedule statistics */
export interface ScheduleStats {
  total_executions: number;
  successful_executions: number;
  failed_executions: number;
  skipped_executions: number;
  avg_duration_ms?: number;
  last_execution_status?: ScheduleExecutionStatus;
}

/** Request to create a schedule */
export interface CreateScheduleRequest {
  name: string;
  description?: string;
  cron_expression: string;
  timezone?: string;
  target: ScheduleTarget;
  enabled?: boolean;
  max_concurrent?: number;
  catch_up_missed?: boolean;
  max_catch_up?: number;
  retry_policy?: Record<string, unknown>;
}

/** Request to update a schedule */
export interface UpdateScheduleRequest {
  description?: string;
  cron_expression?: string;
  timezone?: string;
  target?: ScheduleTarget;
  enabled?: boolean;
  max_concurrent?: number;
  catch_up_missed?: boolean;
  max_catch_up?: number;
  retry_policy?: Record<string, unknown>;
}

/** Schedules list response */
export interface SchedulesResponse {
  data: DurableSchedule[];
  total: number;
}

/** Schedule executions list response */
export interface ScheduleExecutionsResponse {
  data: ScheduleExecution[];
  total: number;
}

/** Manual trigger response */
export interface TriggerResponse {
  execution_id: string;
}

export interface EnqueueTaskRequest {
  activity_type: string;
  input: Record<string, unknown>;
  max_attempts?: number;
  priority?: number;
  timeout_seconds?: number;
}

export interface EnqueueTaskResponse {
  task_id: string;
}

// From legacy eval-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Eval domain types matching Rust Eval, EvalCase, EvalRun, EvalCaseResult
// EvalTarget defines how to instantiate sessions for eval cases.
// Resolution: EvalRun.target → EvalCase.target → Eval.target → org default harness.
export type EvalTarget =
  | {
      type: "session";
      harness_id?: string;
      harness_name?: string;
      agent_id?: string;
      model_id?: string;
      system_prompt?: string;
      max_iterations?: number;
    }
  | {
      type: "app";
      app_id: string;
    }
  | {
      // Label-only target for externally-executed (imported) runs.
      type: "external";
      provider: string;
      model: string;
      params?: Record<string, unknown>;
    };

export interface Eval {
  id: string;
  name: string;
  description?: string;
  target?: EvalTarget;
  model_override?: string;
  tags: string[];
  status: "active" | "archived" | "deleted";
  case_count: number;
  last_run?: EvalRunSummaryView;
  created_at: string;
  updated_at: string;
}

export interface EvalRunSummaryView {
  id: string;
  status: EvalRunStatus;
  summary?: RunSummary;
  created_at: string;
}

export type EvalRunStatus = "pending" | "running" | "completed" | "failed" | "cancelled";

export interface RunSummary {
  total: number;
  passed: number;
  failed: number;
  errored: number;
  pass_rate: number;
  avg_score: number;
  avg_turns: number;
  avg_latency_ms: number;
  total_input_tokens: number;
  total_output_tokens: number;
}

export interface EvalCase {
  id: string;
  name: string;
  description?: string;
  target?: EvalTarget;
  tags: string[];
  conversation: EvalInputMessage[];
  artifacts?: ArtifactSpec[];
  scorers: Scorer[];
  max_turns?: number;
  timeout_seconds?: number;
  position: number;
  created_at: string;
  updated_at: string;
}

export interface EvalInputMessage {
  content: string;
}

export interface ArtifactSpec {
  name: string;
  path: string;
}

export type Scorer =
  | {
      type: "contains";
      text: string;
      weight?: number;
    }
  | {
      type: "not_contains";
      text: string;
      weight?: number;
    }
  | {
      type: "regex";
      pattern: string;
      weight?: number;
    }
  | {
      type: "tool_called";
      tool: string;
      min?: number;
      weight?: number;
    }
  | {
      type: "tool_not_called";
      tool: string;
      weight?: number;
    }
  | {
      type: "tool_call_count";
      min?: number;
      max?: number;
      weight?: number;
    }
  | {
      type: "turns_within";
      max: number;
      weight?: number;
    }
  | {
      type: "file_contains";
      path: string;
      text: string;
      weight?: number;
    }
  | {
      type: "json_schema";
      schema: object;
      weight?: number;
    };

// Provenance for an externally-executed (imported) run. Open-vocab; the
// producing system (e.g. Mira) fills what it has.
export interface EvalRunAttribution {
  system?: string;
  version?: string | null;
  url?: string | null;
  run_id?: string;
  metadata?: Record<string, unknown> | null;
}

export interface EvalRun {
  id: string;
  // "internal" = everruns executed it; "external" = imported from another eval
  // system. Absent on older runs ⇒ treat as internal.
  source?: "internal" | "external";
  attribution?: EvalRunAttribution;
  target?: EvalTarget;
  model_override?: string;
  filter_tags?: string[];
  status: EvalRunStatus;
  triggered_by: string;
  started_at?: string;
  completed_at?: string;
  summary?: RunSummary;
  results: EvalCaseResult[];
  created_at: string;
  updated_at: string;
}

// One named score. Stored as an ordered array (named, attributed) by both the
// external-score write-back and external imports; `scorer`/`na` are present on
// imported scores.
export interface EvalScore {
  scorer?: string;
  pass: boolean;
  value: number;
  reason?: string;
  na?: boolean;
}

// Normalized, provider-agnostic transcript for an externally-executed result.
// Rendered by the generic transcript view; native runs link a session instead.
export interface EvalTranscript {
  input?: string[];
  final_response?: string;
  tool_calls?: string[];
  output?: unknown[];
  iterations?: number;
}

// Per-result metadata envelope. External imports stash the transcript and an
// open-vocab metrics bag here (rather than dedicated columns).
export interface EvalResultMetadata {
  transcript?: EvalTranscript;
  metrics?: Record<string, number>;
  [key: string]: unknown;
}

export interface EvalCaseResult {
  id: string;
  eval_case_id: string;
  case_name?: string;
  session_id?: string;
  target?: EvalTarget;
  target_snapshot?: EvalTarget;
  status: "pending" | "running" | "passed" | "failed" | "errored" | "timeout" | "skipped";
  // Array form (named/attributed) is canonical; the legacy keyed-object form is
  // still accepted for older rows.
  scores?: EvalScore[] | Record<string, { pass: boolean; value: number; reason: string }>;
  metadata?: EvalResultMetadata;
  turns?: number;
  latency_ms?: number;
  input_tokens?: number;
  output_tokens?: number;
  error_message?: string;
  artifacts?: Record<string, string>;
  created_at: string;
  updated_at: string;
}

// Read-only share links (see knowledge/evaluation/evals.md, knowledge/execution/public-endpoints.md).
export interface EvalRunShareLink {
  token: string;
  token_prefix: string;
  created_at: string;
}

export interface EvalRunShareStatus {
  active: boolean;
}

// Sanitized attribution shown on a public share.
export interface PublicAttribution {
  system?: string;
  version?: string;
  url?: string;
}

// Anonymous, sanitized view of one run returned by the public share endpoint.
// Omits org/internal ids, session ids, internal targets, attribution labels.
export interface PublicEvalRun {
  id: string;
  status: EvalRunStatus;
  source?: "internal" | "external";
  attribution?: PublicAttribution;
  summary?: RunSummary;
  created_at: string;
  completed_at?: string;
  results: PublicEvalCaseResult[];
}

export interface PublicEvalCaseResult {
  case_name?: string;
  // Only label-only (external) targets are exposed publicly.
  target?: EvalTarget;
  status: EvalCaseResult["status"];
  scores?: EvalScore[] | Record<string, { pass: boolean; value: number; reason: string }>;
  transcript?: EvalTranscript;
  metrics?: Record<string, number>;
  turns?: number;
  latency_ms?: number;
  input_tokens?: number;
  output_tokens?: number;
  error_message?: string;
}

// Request types
export interface CreateEvalRequest {
  name: string;
  description?: string;
  target?: EvalTarget;
  model_override?: string;
  tags?: string[];
}

export interface UpdateEvalRequest {
  name?: string;
  description?: string;
  target?: EvalTarget;
  model_override?: string;
  tags?: string[];
}

export interface CreateEvalCaseRequest {
  name: string;
  description?: string;
  target?: EvalTarget;
  tags?: string[];
  conversation: EvalInputMessage[];
  artifacts?: ArtifactSpec[];
  scorers: Scorer[];
  max_turns?: number;
  timeout_seconds?: number;
  position?: number;
}

export interface CreateEvalRunRequest {
  target?: EvalTarget;
  model_override?: string;
}

// ============================================
// Observer types (online scoring of production sessions)
// ============================================
// Hand-written (not generated): the backend exposes a flattened tagged union for
// scorer configs and a snake_case tagged union for rule scorers. See the
// `/v1/observers` API contract.

export type ObserverStatus = "active" | "paused" | "archived";

/** Match rules deciding which production turns an observer samples. */
export interface ObserverMatch {
  agent_ids?: string[];
  harness_ids?: string[];
  session_tags?: string[];
}

/**
 * Deterministic rule scorer variants (snake_case, discriminated by `type`).
 * Mirrors the eval Scorer union, minus `file_contains` (rejected by the server
 * for observers). `weight` defaults to 1.0 server-side.
 */
export type ScorerRule =
  | { type: "contains"; text: string; weight?: number }
  | { type: "not_contains"; text: string; weight?: number }
  | { type: "regex"; pattern: string; weight?: number }
  | { type: "tool_called"; tool: string; min?: number; weight?: number }
  | { type: "tool_not_called"; tool: string; weight?: number }
  | { type: "tool_call_count"; min?: number; max?: number; weight?: number }
  | { type: "turns_within"; max: number; weight?: number }
  | { type: "json_schema"; schema: object; weight?: number };

/** Scope a scorer evaluates against. Only `turn` is currently supported. */
export type ObserverScorerScope = "turn" | "session" | "tool";

/**
 * Per-scorer configuration. FLATTENED tagged union discriminated by `method`:
 * the method-specific fields sit alongside `key`/`scope`, not nested.
 */
export type ObserverScorerConfig =
  | {
      key: string;
      scope: ObserverScorerScope;
      method: "rule";
      rule: ScorerRule;
    }
  | {
      key: string;
      scope: ObserverScorerScope;
      method: "llm_judge";
      rubric: string;
      model_id?: string;
      pass_threshold: number;
    };

export interface Observer {
  id: string;
  name: string;
  description?: string;
  match: ObserverMatch;
  sampling_rate: number;
  scorers: ObserverScorerConfig[];
  status: ObserverStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string;
}

export interface CreateObserverRequest {
  name: string;
  description?: string;
  match?: ObserverMatch;
  sampling_rate?: number;
  scorers: ObserverScorerConfig[];
}

export interface UpdateObserverRequest {
  name?: string;
  description?: string;
  match?: ObserverMatch;
  sampling_rate?: number;
  scorers?: ObserverScorerConfig[];
  status?: ObserverStatus;
}

export type TraceScoreStatus = "pending" | "scoring" | "completed" | "errored" | "skipped";

/** A single scored production turn produced by an observer. */
export interface TraceScore {
  id: string;
  observer_id: string;
  scorer_key: string;
  session_id: string;
  turn_id: string;
  agent_id?: string;
  agent_version_id?: string;
  harness_id?: string;
  status: TraceScoreStatus;
  pass?: boolean;
  value?: number;
  label?: string;
  reason?: string;
  judge_input_tokens?: number;
  judge_output_tokens?: number;
  judge_cost_usd?: number;
  error_message?: string;
  created_at: string;
  updated_at: string;
}

// From legacy event-types.ts; retained as UI compatibility over generated OpenAPI schemas.
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
  /** Stable public ID for this assistant message across its streaming lifecycle */
  message_id: string;
  model?: string;
  /** 1-based iteration number within the turn */
  iteration?: number;
  phase?: "commentary" | "final_answer";
}

/** Data for output.message.delta event (streaming text update) */
export interface OutputMessageDeltaData {
  turn_id: string;
  /** Stable public ID for this assistant message across its streaming lifecycle */
  message_id: string;
  /** The new text since last delta */
  delta: string;
  /** Accumulated text so far */
  accumulated: string;
  phase?: "commentary" | "final_answer";
}

/** Data for output.message.replaced event (guardrail replacement) */
export interface LegacyOutputMessageReplacedData {
  turn_id: string;
  message_id: string;
  guardrail_capability_id: string;
  guardrail_id: string;
  reason_code: string;
  replacement: string;
}

/** Data for output.message.completed event */
export interface OutputMessageCompletedData {
  message: Message;
  metadata?: ModelMetadata;
  usage?: TokenUsage;
  error_code?: string;
  error_fields?: Record<string, unknown>;
  /** Error-disclosure mode applied to error_code/error_fields */
  error_disclosure?: string;
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
  /** Canonical assistant message emitted by output.message.completed */
  final_message_id?: string;
  /** Bounded preview of the final visible assistant answer */
  final_answer_preview?: string;
  /** First-token latency for the turn */
  time_to_first_token_ms?: number;
  /** Number of tool calls completed during the turn */
  tool_call_count?: number;
  /** Number of LLM generation calls executed during the turn */
  llm_call_count?: number;
  /** Optional explicit completion status */
  status?: string;
}

/** Data for turn.failed event */
export interface TurnFailedData {
  turn_id: string;
  error: string;
  error_code?: string;
  error_fields?: Record<string, unknown>;
  /** Error-disclosure mode applied to error_code/error_fields */
  error_disclosure?: string;
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
  /** Human-readable narration after the call completes */
  completed_narration?: string;
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
  /** Stable fingerprint of tool name + normalized arguments */
  tool_call_fingerprint?: string;
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
  completed_headline?: string;
}

/** Data for tool.progress event (interim status during execution) */
export interface ToolProgressData {
  tool_call_id: string;
  tool_name: string;
  /** Human-readable status message (e.g., "Connecting to browser…") */
  message: string;
  display_name?: string;
}

/** Data for tool.output.delta event (streamed output chunks during execution) */
export interface ToolOutputDeltaData {
  tool_call_id: string;
  tool_name: string;
  /** Incremental output chunk */
  delta: string;
  /** Output stream identifier (e.g., "stdout", "stderr") */
  stream: string;
}

/** Data for tool.completed event */
export interface ToolCompletedData {
  tool_call_id: string;
  tool_name: string;
  /** Stable fingerprint of tool name + normalized arguments */
  tool_call_fingerprint?: string;
  /** Stable fingerprint of tool name + normalized result/error */
  tool_result_fingerprint?: string;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  success: boolean;
  status: "success" | "error" | "timeout" | "cancelled";
  result?: ContentPart[];
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
  retry?: LlmRetryInfo | null;
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

/** Data for reason.item event (durable opaque assistant reasoning response item).
 *
 * Carries provider-supplied opaque reasoning artifacts plus curated summary
 * text. Plaintext hidden chain-of-thought is intentionally not persisted on
 * this event — see knowledge/execution/events.md.
 */
export interface ReasonItemData {
  turn_id: string;
  /** Provider that produced the reasoning item (e.g., "openai"). */
  provider: string;
  /** Model identifier reported by the provider, when known. */
  model?: string;
  /** Provider-assigned identifier for the reasoning item. */
  item_id: string;
  /** Provider-encrypted reasoning context, when supplied. Opaque to consumers. */
  encrypted_content?: string;
  /** Safe summary text segments curated by the provider. */
  summary?: string[];
  /** Per-item reasoning token count, when the provider reports one. */
  token_count?: number;
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

/** Data for session.model.changed event (answering model switched between turns) */
export interface SessionModelChangedData {
  /** Absent when the previous turn ran on a default the server could not name. */
  previous_model_id?: string;
  previous_model_name?: string;
  model_id: string;
  model_name: string;
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
  | LegacyOutputMessageReplacedData
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
  | ToolOutputDeltaData
  | ToolCompletedData
  | LlmGenerationData
  | SessionStartedData
  | SessionActivatedData
  | SessionIdledData
  | ReasonThinkingStartedData
  | ReasonThinkingDeltaData
  | ReasonThinkingCompletedData
  | ReasonItemData
  | ContextCompactingData
  | ContextCompactedData
  | SessionModelChangedData
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
  "output.message.replaced": LegacyOutputMessageReplacedData;
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
  "tool.output.delta": ToolOutputDeltaData;
  "tool.completed": ToolCompletedData;
  "llm.generation": LlmGenerationData;
  "session.started": SessionStartedData;
  "session.activated": SessionActivatedData;
  "session.idled": SessionIdledData;
  "reason.thinking.started": ReasonThinkingStartedData;
  "reason.thinking.delta": ReasonThinkingDeltaData;
  "reason.thinking.completed": ReasonThinkingCompletedData;
  "reason.item": ReasonItemData;
  "context.compacting": ContextCompactingData;
  "context.compacted": ContextCompactedData;
  "session.model.changed": SessionModelChangedData;
}

export type KnownEventType = keyof EventTypeMap;

/** Type predicate: narrows an Event to one with a known type and correctly-typed data. */
export function isEventOfType<T extends KnownEventType>(
  event: Event,
  type: T,
): event is Event & {
  type: T;
  data: EventTypeMap[T];
} {
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

// From legacy image-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Image types (for message attachments)
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
export const ALLOWED_IMAGE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/webp"] as const;

export type AllowedImageType = (typeof ALLOWED_IMAGE_TYPES)[number];

/** Maximum image size in bytes (100 MB) */
export const MAX_IMAGE_SIZE = 100 * 1024 * 1024;

// From legacy knowledge-index-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Knowledge Index types
//
// Mirrors crates/server/src/domains/knowledge_indexes/types.rs:
// KnowledgeIndexResponse / KnowledgeIndexDocumentResponse and the request bodies.
export type KnowledgeIndexStatus = "active" | "archived" | "deleted";

export type KnowledgeIndexSourceType = "github" | "git";

export type KnowledgeIndexSyncStatus = "idle" | "pending" | "syncing" | "synced" | "failed";

export interface KnowledgeIndex {
  id: string;
  name: string;
  description?: string | null;
  source_type: KnowledgeIndexSourceType;
  /** Non-secret source coordinates. Never holds credentials. */
  source_config: Record<string, unknown>;
  embedding_model_id: string;
  vector_dim?: number | null;
  document_count: number;
  status: KnowledgeIndexStatus;
  sync_status: KnowledgeIndexSyncStatus;
  last_synced_at?: string | null;
  last_sync_error?: string | null;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export interface KnowledgeIndexDocument {
  id: string;
  index_id: string;
  source_uri: string;
  title?: string | null;
  mime_type?: string | null;
  content_hash?: string | null;
  size_bytes?: number | null;
  chunk_count: number;
  last_seen_at?: string | null;
  created_at: string;
  updated_at: string;
}

export interface CreateKnowledgeIndexRequest {
  name: string;
  description?: string;
  /** Defaults to "github" on the server. */
  source_type?: KnowledgeIndexSourceType;
  source_config?: Record<string, unknown>;
  /** Required. Must resolve to a model whose driver declares Embeddings. */
  embedding_model_id: string;
}

export interface UpdateKnowledgeIndexRequest {
  name?: string;
  /** Set to null to clear the description. */
  description?: string | null;
  source_config?: Record<string, unknown>;
  /** Required on the server; cannot be cleared. */
  embedding_model_id?: string;
}

export interface ListKnowledgeIndexesParams {
  includeArchived?: boolean;
  search?: string;
}

// From legacy mcp-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// MCP Server types
/** MCP Server transport type */
export type McpServerTransportType = "http";

/** MCP Server auth mode */
export type McpServerAuthMode = "none" | "api_key" | "oauth";

/** MCP Server status */
export type McpServerStatus = "active" | "disabled" | "archived" | "deleted";

/**
 * MCP protocol-era adoption policy. Pinned values are the spec version dates.
 * - `auto`: probe and adapt across every era — the default.
 * - `2025-03-26`: stateful handshake + session id.
 * - `2025-06-18`: stateful handshake + session id.
 * - `2026-07-28`: stateless (no handshake).
 */
export type McpProtocolMode = "auto" | "2025-03-26" | "2025-06-18" | "2026-07-28";

/** MCP Server configuration */
export interface McpServer {
  id: string;
  name: string;
  description: string | null;
  url: string;
  transport_type: McpServerTransportType;
  status: McpServerStatus;
  auth_mode: McpServerAuthMode;
  /** Protocol-era policy. Omitted by the API when `auto` (the default). */
  protocol_mode?: McpProtocolMode;
  oauth_provider_id?: string;
  api_key_set: boolean;
  headers: Record<string, string>;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
}

/** Request to create an MCP server */
export interface CreateMcpServerRequest {
  name: string;
  description?: string;
  url: string;
  transport_type?: McpServerTransportType;
  auth_mode?: McpServerAuthMode;
  protocol_mode?: McpProtocolMode;
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
  auth_mode?: McpServerAuthMode;
  protocol_mode?: McpProtocolMode;
  api_key?: string;
  headers?: Record<string, string>;
}

// From legacy memory-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Memory types
export type MemoryStatus = "active" | "archived" | "deleted";

export interface Memory {
  id: string;
  name: string;
  description?: string | null;
  source_type: MemorySourceType;
  source: MemorySource;
  is_readonly: boolean;
  sync_status: MemorySyncStatus;
  last_synced_at?: string | null;
  last_sync_error?: string | null;
  status: MemoryStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
  deleted_at?: string | null;
}

export type MemorySourceType = "manual" | "github" | "git";

export type MemorySyncStatus = "idle" | "pending" | "syncing" | "synced" | "failed";

export type MemorySource =
  | {
      provider: "manual";
    }
  | {
      provider: "github";
      repository: string;
      branch: string;
      root_folder?: string | null;
      sync_interval_secs?: number | null;
    }
  | {
      provider: "git";
      url: string;
      branch: string;
      root_folder?: string | null;
      sync_interval_secs?: number | null;
    };

export type CreateMemorySource =
  | {
      type: "github";
      repository: string;
      branch?: string;
      root_folder?: string;
      sync_interval_secs?: number;
    }
  | {
      type: "git";
      url: string;
      branch?: string;
      root_folder?: string;
      sync_interval_secs?: number;
    };

export interface CreateMemoryRequest {
  name: string;
  description?: string;
  source?: CreateMemorySource;
}

export interface UpdateMemoryRequest {
  name?: string;
  description?: string | null;
  source?: CreateMemorySource;
}

export interface ListMemoriesParams {
  includeArchived?: boolean;
  search?: string;
}

// From legacy message-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Message types and content parts
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
  | {
      type: "text";
      text: string;
      /** Claim-level citations attached to spans of `text` (see knowledge/runtime-resources/citations.md). */
      annotations?: import("./schema-types").TextAnnotation[];
    }
  | {
      type: "image";
      url?: string;
      base64?: string;
      media_type?: string;
    }
  | {
      type: "image_file";
      image_id: string;
      filename?: string;
    }
  | {
      type: "resource";
      uri?: string;
      mimeType?: string;
      mime_type?: string;
      text?: string;
      resource?: {
        uri?: string;
        mimeType?: string;
        mime_type?: string;
        text?: string;
      };
    }
  | {
      type: "tool_call";
      id: string;
      name: string;
      arguments: Record<string, unknown>;
    }
  | {
      type: "tool_result";
      tool_call_id: string;
      result?: unknown;
      error?: string;
    };

// Helper type guards for ContentPart
export function isTextPart(part: ContentPart): part is {
  type: "text";
  text: string;
  annotations?: import("./schema-types").TextAnnotation[];
} {
  return part.type === "text";
}

export function isToolCallPart(part: ContentPart): part is {
  type: "tool_call";
  id: string;
  name: string;
  arguments: Record<string, unknown>;
} {
  return part.type === "tool_call";
}

export function isToolResultPart(part: ContentPart): part is {
  type: "tool_result";
  tool_call_id: string;
  result?: unknown;
  error?: string;
} {
  return part.type === "tool_result";
}

export function isImageFilePart(part: ContentPart): part is {
  type: "image_file";
  image_id: string;
  filename?: string;
} {
  return part.type === "image_file";
}

export function isResourcePart(part: ContentPart): part is Extract<
  ContentPart,
  {
    type: "resource";
  }
> {
  return part.type === "resource";
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
  /**
   * Generic client hints -- arbitrary key-value pairs declared by the client.
   * Session-level defaults are set at session creation; per-message values
   * override session hints key-by-key (shallow merge).
   *
   * Examples: `{ setup_connection: true, rich_media: true }`
   */
  hints?: Record<string, unknown>;
  /** Locale override for this message turn (BCP 47, e.g. `uk-UA`) */
  locale?: string;
}

/**
 * Message for UI display
 *
 * Uses DisplayMessageRole since messages can be derived from events
 * including tool.completed events which become "tool_result" messages.
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
  /** Execution phase: "in_progress" (intermediate, has tool calls) or "completed" (final answer) */
  phase?: string;
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
  // Optional active agent participant to address for this turn. When omitted,
  // the session host remains the responder (default 1:1 behavior).
  addressed_participant_id?: string | null;
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
    .map((part) => part.text)
    .join("\n");
}

// Helper function to get tool calls from content parts
export function getToolCallsFromContent(content: ContentPart[]): Array<{
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}> {
  return content.filter(isToolCallPart).map((part) => ({
    id: part.id,
    name: part.name,
    arguments: part.arguments,
  }));
}

// From legacy payment-types.ts; retained as UI compatibility over generated OpenAPI schemas.
export type PaymentRail = "mpp_tempo" | "x402_base";

export type PaymentOwnerType = "user" | "agent_identity" | "organization";

export type PaymentStatus = "active" | "disabled" | "pending" | "succeeded" | "failed" | "released";

export interface PaymentAccount {
  id: string;
  organization_id: string;
  owner_type: PaymentOwnerType;
  owner_id: string;
  rail: PaymentRail;
  label: string;
  public_address?: string | null;
  status: PaymentStatus;
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface CreatePaymentAccountRequest {
  owner_type: PaymentOwnerType;
  owner_id: string;
  rail: PaymentRail;
  label: string;
  public_address?: string | null;
  private_key?: string;
  metadata?: Record<string, unknown>;
}

export interface UpdatePaymentAccountRequest {
  label?: string;
  public_address?: string | null;
  private_key?: string;
  status?: "active" | "disabled";
  metadata?: Record<string, unknown>;
}

export interface PaymentPolicy {
  id: string;
  organization_id: string;
  payment_account_id: string;
  subject_type: "user" | "agent_identity" | "agent" | "app" | "session" | "org";
  subject_id: string;
  allowed_capabilities: string[];
  allowed_hosts: string[];
  rail_preference: PaymentRail[];
  max_amount_usd_per_request?: number | null;
  max_amount_usd_per_turn?: number | null;
  max_amount_usd_per_day?: number | null;
  require_approval_above_usd?: number | null;
  status: PaymentStatus;
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface CreatePaymentPolicyRequest {
  payment_account_id: string;
  subject_type: PaymentPolicy["subject_type"];
  subject_id: string;
  allowed_capabilities: string[];
  allowed_hosts: string[];
  rail_preference: PaymentRail[];
  max_amount_usd_per_request?: number | null;
  max_amount_usd_per_turn?: number | null;
  max_amount_usd_per_day?: number | null;
  require_approval_above_usd?: number | null;
  metadata?: Record<string, unknown>;
}

export interface UpdatePaymentPolicyRequest {
  allowed_capabilities?: string[];
  allowed_hosts?: string[];
  rail_preference?: PaymentRail[];
  max_amount_usd_per_request?: number | null;
  max_amount_usd_per_turn?: number | null;
  max_amount_usd_per_day?: number | null;
  require_approval_above_usd?: number | null;
  status?: "active" | "disabled";
  metadata?: Record<string, unknown>;
}

export interface PaymentAttempt {
  id: string;
  organization_id: string;
  payment_account_id?: string | null;
  session_id?: string | null;
  capability: string;
  operation: string;
  rail?: PaymentRail | null;
  amount_usd: number;
  currency: string;
  target_url: string;
  request_hash?: string | null;
  status: PaymentStatus;
  error_message?: string | null;
  receipt: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

// From legacy plugin-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Plugin Marketplace and Installed Plugin types
// See knowledge/integrations/plugins.md for the data model and API sketch.
// ============================================
// Marketplace types
// ============================================
/** Status of a registered marketplace */
export type MarketplaceStatus = "active" | "disabled";

/** Where a marketplace's catalog is fetched from. `local_path` is dev-only. */
export type MarketplaceSourceType = "github" | "url" | "local_path";

/** Source configuration; the populated field follows `source_type` */
export interface MarketplaceSource {
  /** GitHub repo `owner/repo` (source_type "github") */
  repo?: string;
  /** Direct HTTPS URL to a marketplace.json (source_type "url") */
  url?: string;
  /** Local filesystem path (source_type "local_path", dev-only) */
  path?: string;
}

/** A registered plugin marketplace (org-scoped catalog) */
export interface Marketplace {
  id: string;
  name: string;
  source_type: MarketplaceSourceType;
  source: MarketplaceSource;
  status: MarketplaceStatus;
  /** ISO timestamp of the last successful sync */
  last_synced_at: string | null;
  /** Resolved commit SHA if source is a git repo */
  last_synced_sha: string | null;
  created_at: string;
  updated_at: string;
}

/** Request to register a new marketplace */
export interface CreateMarketplaceRequest {
  name: string;
  source_type: MarketplaceSourceType;
  /** `owner/repo` for github, HTTPS URL for url, filesystem path for local_path */
  source: string;
}

/** Request to update a marketplace */
export interface UpdateMarketplaceRequest {
  name?: string;
  status?: MarketplaceStatus;
}

// ============================================
// Marketplace catalog entry types
// ============================================
/** A plugin entry as it appears in a marketplace's synced catalog */
export interface MarketplaceCatalogEntry {
  /** Plugin name (kebab-case) */
  name: string;
  /** Human-readable display name */
  display_name: string | null;
  description: string | null;
  version: string | null;
  author: string | null;
  category: string | null;
  /** Whether this plugin is already installed in the current org */
  installed: boolean;
}

// ============================================
// Installed plugin types
// ============================================
/** Status of an installed plugin */
export type InstalledPluginStatus = "active" | "disabled";

/** Install warning about unsupported plugin components (e.g. hooks, lspServers) */
export type InstalledPluginWarning = string;

/** An installed plugin (compiled into the capability registry) */
export interface InstalledPlugin {
  id: string;
  /** Kebab-case plugin name used for display and discovery */
  name: string;
  display_name: string | null;
  description: string | null;
  version: string | null;
  /** Pinned commit SHA at install time */
  pinned_sha: string | null;
  /** Name of the marketplace this was installed from */
  marketplace: string | null;
  /** Stable capability reference: `plugin:{install_id}` */
  capability_ref: string;
  status: InstalledPluginStatus;
  /** Install-time warnings for unsupported plugin components */
  warnings: InstalledPluginWarning[];
  /** True when the marketplace catalog has a newer version or SHA */
  update_available: boolean;
  created_at: string;
  updated_at: string;
}

/** Request to install a plugin from a marketplace catalog entry */
export interface InstallPluginRequest {
  marketplace_id: string;
  plugin_name: string;
}

/** Request to update an installed plugin's metadata */
export interface UpdateInstalledPluginRequest {
  status?: InstalledPluginStatus;
}

// From legacy provider-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// LLM Provider and Model types
// ============================================
// LLM Provider types
// ============================================
export type DriverId =
  | "openai"
  | "openrouter"
  | "azure_openai"
  | "openai_completions"
  | "anthropic"
  | "gemini"
  | "bedrock"
  | "mai"
  | "fireworks"
  | "meta";

export type ProviderStatus = "active" | "disabled";

/** Vendor/brand of a model, derived from the backend model registry. */
export type ModelVendor =
  | "openai"
  | "anthropic"
  | "google"
  | "nvidia"
  | "qwen"
  | "microsoft"
  | "meta"
  | "minimax"
  | "moonshot"
  | "xai"
  | "llmsim";

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

/**
 * Per-connection request options: extra headers sent with every request to the
 * provider, and the prompt-cache diagnostics opt-in (honored by drivers with a
 * diagnostics protocol, today Anthropic).
 */
export interface ProviderRequestOptions {
  headers?: ProviderRequestHeader[];
  cache_diagnostics?: boolean;
}

export interface Provider {
  id: string;
  name: string;
  provider_type: DriverId;
  base_url?: string;
  api_key_set: boolean;
  status: ProviderStatus;
  /** Whether this provider is host-managed (read-only to org admins). */
  managed: boolean;
  created_at: string;
  updated_at: string;
  /** Resolved trace/observability link configuration (driver defaults + overrides). */
  trace?: ProviderTraceConfig;
  /** Extra headers and diagnostics options applied to every request to this provider. */
  request_options?: ProviderRequestOptions;
}

export interface Model {
  id: string;
  provider_id: string;
  model_id: string;
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

export interface CreateProviderRequest {
  name: string;
  provider_type: DriverId;
  base_url?: string;
  api_key?: string;
  /** Typed credential fields keyed by the driver's credential-schema field names. */
  credentials?: Record<string, string>;
  trace?: ProviderTraceConfig;
  request_options?: ProviderRequestOptions;
}

export interface UpdateProviderRequest {
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
  driver: DriverId;
  credential_schema: CredentialFormSchema;
  supports_oauth: boolean;
}

/** Response shape of `GET /v1/providers/config`. */
export interface ProvidersConfigResponse {
  policies: Record<string, boolean>;
  drivers: DriverCredentialInfo[];
}

export interface CreateModelRequest {
  model_id: string;
  display_name: string;
  capabilities?: string[];
  enabled?: boolean;
  is_favorite?: boolean;
}

export interface UpdateModelRequest {
  provider_id?: string;
  model_id?: string;
  display_name?: string;
  capabilities?: string[];
  enabled?: boolean;
  is_favorite?: boolean;
}

// Response from syncing models from a provider
export type SyncModelsResponse =
  | {
      status: "success";
      created: number;
      updated: number;
      stale: number;
    }
  | {
      status: "not_supported";
    };

// From legacy reporting-types.ts; retained as UI compatibility over generated OpenAPI schemas.
export type ReportColumnKind = "dimension" | "measure";

export type ReportFilterOp = "eq" | "neq" | "in";

export type ReportOrderDirection = "asc" | "desc";

export type ReportExportFormat = "csv" | "json";

export interface ReportTimeRange {
  from: string;
  to: string;
}

export interface ReportFilter {
  field: string;
  op: ReportFilterOp;
  value: unknown;
}

export interface ReportOrderBy {
  dimension?: string;
  measure?: string;
  direction?: ReportOrderDirection;
}

export interface ReportQuery {
  dataset: string;
  time_range: ReportTimeRange;
  dimensions: string[];
  measures: string[];
  filters?: ReportFilter[];
  order_by?: ReportOrderBy[];
  limit?: number;
}

export interface ReportColumn {
  name: string;
  kind: ReportColumnKind;
}

export interface ReportResult {
  as_of: string;
  freshness_lag_ms?: number | null;
  columns: ReportColumn[];
  rows: Record<string, unknown>[];
}

export interface DatasetCatalogEntry {
  name: string;
  dimensions: string[];
  measures: string[];
  filter_fields: string[];
}

export interface DatasetCatalog {
  datasets: DatasetCatalogEntry[];
}

export interface SavedReportDashboardMetadata {
  title?: string;
  section?: string;
  chart_type?: string;
  position?: number;
}

export interface SavedReport {
  id: string;
  name: string;
  description?: string;
  query: ReportQuery;
  dashboard?: SavedReportDashboardMetadata;
  created_at: string;
  updated_at: string;
}

export interface CreateSavedReportRequest {
  name: string;
  description?: string;
  query: ReportQuery;
  dashboard?: SavedReportDashboardMetadata;
}

export interface ReportExport {
  format: ReportExportFormat;
  filename: string;
  content_type: string;
  content: string;
  as_of: string;
  freshness_lag_ms?: number | null;
}

export interface DatasetProjectorLag {
  dataset: string;
  latest_projected_at?: string | null;
  freshness_lag_ms?: number | null;
}

export interface FailedReportingOutboxRow {
  id: string;
  org_id: number;
  source_type: string;
  source_id: string;
  attempts: number;
  last_error?: string | null;
  updated_at: string;
}

export interface ReportingOutboxDiagnostics {
  pending: number;
  processing: number;
  failed: number;
  completed: number;
  oldest_pending_at?: string | null;
  failed_rows: FailedReportingOutboxRow[];
}

export interface ReportingDiagnostics {
  generated_at: string;
  projector_lag: DatasetProjectorLag[];
  outbox: ReportingOutboxDiagnostics;
}

export interface ProjectorRunResult {
  claimed: number;
  completed: number;
  failed: number;
}

export interface ReportingBackfillResult {
  enqueued: number;
  events: number;
  sessions: number;
  llm_generations: number;
  usage_ledger: number;
}

// From legacy session-types.ts; retained as UI compatibility over generated OpenAPI schemas.
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
  /**
   * Workspace this session is attached to (wsp_<32-hex>); owns the virtual
   * filesystem. For the default 1:1 case this equals the session id. Optional
   * here until the UI consumes it directly instead of deriving from the id.
   */
  workspace_id?: string;
  harness_id: string;
  agent_id: string | null;
  /** Immutable agent version captured when the session was created or rebound. */
  agent_version_id?: string | null;
  agent_identity_id?: string | null;
  owner_principal_id: string;
  resolved_owner_user_id?: string | null;
  owner?: PrincipalSummary | null;
  effective_owner?: PrincipalSummary | null;
  title: string | null;
  /** Objective shown in session lists and injected into the agent prompt */
  goal?: string | null;
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
  /**
   * How the session was started (EVE-852). Server-owned for every ingress
   * except `chat`/`api`, which is what makes the Sessions source facet
   * trustworthy. Optional here so responses from an older server still parse.
   */
  source?: SessionSource;
  /**
   * Outcome-oriented view of the session, derived server-side from its status
   * and the outcome of its most recent turn. `status` alone has no notion of
   * failure, which is why the list and the facet rail read this instead.
   */
  activity?: SessionActivity;
  /**
   * Generated one-sentence description of what the run did, and where it
   * failed (EVE-867). Absent for chat threads, for runs with no terminal turn
   * yet, and on deployments with no utility LLM — readers must have a fallback.
   */
  run_summary?: string;
  created_at: string;
  updated_at: string;
  started_at: string | null;
  finished_at: string | null;
  /** Cumulative token usage for all LLM calls in this session */
  usage?: TokenUsage;
  /** Whether this session is pinned by the current user */
  is_pinned?: boolean;
  /**
   * When this session was archived; absent means active. Archived sessions are
   * hidden from default list results and shown by passing
   * `include_archived=true`. Unlike `is_pinned`, archive is a property of the
   * session itself rather than of the viewer.
   */
  archived_at?: string | null;
  /** Number of active schedules for this session */
  active_schedule_count?: number;
  /**
   * Total events recorded for this session. Denormalized counter, omitted when
   * zero — an empty tab carries no badge (EVE-868).
   */
  event_count?: number;
  /**
   * Background work the session owns — subagents, external agents, background
   * tools. This is what the Work tab holds; `active_schedule_count` describes
   * only the schedules it also lists. Omitted when zero.
   */
  task_count?: number;
  /**
   * Non-directory files in this session's workspace. Persisted files only:
   * capability-provided virtual mounts are served from memory and excluded.
   * Omitted when zero.
   */
  file_count?: number;
  /** Aggregated UI features from all active capabilities (harness + agent + session) */
  features?: string[];
  /** Session-level system prompt override (prepended to agent prompt) */
  system_prompt?: string | null;
  /** Session-level initial files (additive to agent initial_files) */
  initial_files?: InitialFile[];
  /** Session-level client hints (defaults for every turn) */
  hints?: Record<string, unknown>;
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList | null;
  /**
   * Session this one was forked from (knowledge/runtime-resources/forking-sessions.md).
   * NULL for sessions that were not forked. Distinct from a subagent's parent.
   */
  forked_from_session_id?: string | null;
  /** Parent event sequence the fork was taken at. NULL unless this is a fork. */
  forked_from_sequence?: number | null;
}

/** Session counts grouped by status */
export interface SessionStats {
  total: number;
  active: number;
  idle: number;
  started: number;
  waiting_for_tool_results: number;
}

/** Aggregate usage stats for an agent or harness detail page. */
export interface ResourceStats {
  session_count: number;
  active_session_count: number;
  idle_session_count: number;
  started_session_count: number;
  waiting_for_tool_results_session_count: number;
  execution_count: number;
  total_session_duration_ms: number;
  avg_session_duration_ms?: number | null;
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_read_tokens: number;
  total_cache_creation_tokens: number;
  total_actual_cost_usd: number;
  total_estimated_cost_usd: number;
  total_cost_usd: number;
  first_session_at?: string | null;
  last_session_at?: string | null;
  last_execution_at?: string | null;
}

export interface CreateSessionRequest {
  /** Harness ID for this session. If omitted, the harness is derived from the agent (when one is supplied), else the org default harness. */
  harness_id?: string;
  /** Harness name, resolved within the org. Mutually exclusive with `harness_id`. */
  harness_name?: string;
  /** Agent ID to work in this session (optional). Mutually exclusive with `agent_name`. */
  agent_id?: string;
  /** Agent name to work in this session (optional). Mutually exclusive with `agent_id`. */
  agent_name?: string;
  agent_identity_id?: string;
  title?: string;
  locale?: string;
  tags?: string[];
  model_id?: string;
  /** Session-level system prompt override (prepended to agent prompt) */
  system_prompt?: string | null;
  /** Session-level initial files (additive to agent initial_files) */
  initial_files?: InitialFile[];
  /**
   * Session-level client hints -- arbitrary key-value pairs that tell the
   * server what the client can handle. Per-message `controls.hints` override
   * these key-by-key (shallow merge).
   */
  hints?: Record<string, unknown>;
  /** Network access list for URL filtering */
  network_access?: NetworkAccessList;
}

export interface UpdateSessionRequest {
  title?: string;
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
  owner_principal_id: string;
  resolved_owner_user_id?: string | null;
  owner?: PrincipalSummary | null;
  effective_owner?: PrincipalSummary | null;
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
// Session Resource Registry types
// ============================================
export type SessionResourceStatus = "active" | "completed" | "failed" | "released";

/** A resource registered in the session resource registry. */
export interface SessionResourceEntry {
  /** Caller-provided stable ID (unique per session). */
  resource_id: string;
  session_id: string;
  /** Resource kind: "sandbox", "subagent", "browser_session", etc. */
  kind: string;
  /** Human-readable label. */
  display_name: string;
  status: SessionResourceStatus;
  /** Kind-specific non-secret metadata. */
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

// ============================================
// Session Task types
// ============================================
// Lifecycle state of a session task. Three classes: active (queued, running),
// interrupted (awaiting_input, resumable), terminal (succeeded, failed,
// canceled). Timeout/rejection are error.kind values on "failed", not states.
export type SessionTaskState =
  | "queued"
  | "running"
  | "awaiting_input"
  | "succeeded"
  | "failed"
  | "canceled";

/** True when the task state is terminal (succeeded, failed, canceled). */
export function isTerminalTaskState(state: SessionTaskState): boolean {
  return state === "succeeded" || state === "failed" || state === "canceled";
}

/** Progress shape shared with background tool execution. */
export interface TaskProgress {
  current?: number;
  total?: number;
  unit?: string;
  label?: string;
}

/** Structured ask posted by a task that needs input to continue. */
export interface TaskInputRequest {
  /** Stable ID referenced by the answering message's `in_reply_to`. */
  id: string;
  /** Human/agent-readable prompt. */
  prompt: string;
  /** Optional machine-readable description of the expected answer. */
  expected?: unknown;
}

/** Terminal error detail. Timeout/rejection/orphaned are kinds, not states. */
export interface TaskError {
  kind: string;
  message: string;
}

/** Typed link to something the task produced. */
export interface TaskArtifact {
  name: string;
  /** Artifact type: "file", "url", "session", "pr", etc. */
  type: string;
  /** Session VFS path, when the artifact lives in the session filesystem. */
  path?: string;
  /** External URL, when the artifact lives elsewhere. */
  url?: string;
}

/** Cross-references owned by a task. */
export interface TaskLinks {
  /** Child session, for subagent-shaped tasks. Full transcript lives there. */
  child_session_id?: string;
  /** Remote task ID, for tasks wrapping an external protocol task (A2A). */
  remote_task_id?: string;
  /** Session resources (sandboxes, browser sessions) this task holds. */
  resource_ids?: string[];
}

/** When outbound task activity wakes the owning session's agent. */
export type TaskWakePolicy = "silent" | "on_terminal" | "on_activity";

/** A unit of background work owned by a session. */
export interface SessionTask {
  /** `task_*` public ID. */
  id: string;
  session_id: string;
  /**
   * Root of the owning session's delegation tree (EVE-680). Surfaced on API
   * reads so cross-session tooling can group a whole tree's tasks by one id.
   * `null`/absent for a top-level session that is its own root.
   */
  root_session_id?: string | null;
  /** Task kind: "subagent", "external_agent", "background_tool", "monitor", etc. */
  kind: string;
  /** Human-readable label. */
  display_name: string;
  /** Kind-specific input (instructions, tool args, external agent id). */
  spec: unknown;
  state: SessionTaskState;
  /** Short live status line ("polling remote task", "iteration 4/10"). */
  state_detail?: string;
  progress?: TaskProgress;
  /** Pending ask while `awaiting_input`; cleared when answered. */
  input_request?: TaskInputRequest;
  /** Cooperative cancel intent. A flag, not a state. */
  cancel_requested_at?: string;
  /** Human-readable outcome. */
  summary?: string;
  /** Machine result in the session VFS: `/.tasks/{task_id}/result.json`. */
  result_path?: string;
  artifacts?: TaskArtifact[];
  error?: TaskError;
  /** Execution attempt, starting at 1. Incremented on re-attach. */
  attempt: number;
  worker_id?: string;
  heartbeat_at?: string;
  links?: TaskLinks;
  wake_policy: TaskWakePolicy;
  created_at: string;
  started_at?: string;
  finished_at?: string;
  updated_at: string;
}

/** One content part of a task message. */
export type TaskMessagePart =
  | {
      type: "text";
      text: string;
    }
  | {
      type: "data";
      data: unknown;
    };

/** A message exchanged between a session and one of its tasks. */
export interface TaskMessage {
  /** `tmsg_*` public ID. */
  id: string;
  task_id: string;
  /** Inbound = session to task. */
  direction: "inbound" | "outbound";
  content: TaskMessagePart[];
  /** Set when this message answers a `TaskInputRequest`. */
  in_reply_to?: string;
  created_at: string;
}

/** Task snapshot plus its recent message thread. */
export interface SessionTaskDetail {
  task: SessionTask;
  messages: TaskMessage[];
}

/** Request to post an inbound message (steering or input answer) to a task. */
export interface PostTaskMessageRequest {
  content: TaskMessagePart[];
  in_reply_to?: string;
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

// From legacy skill-types.ts; retained as UI compatibility over generated OpenAPI schemas.
// Skill types (Agent Skills registry)
/** Skill source type */
export type SkillSourceType = "markdown" | "archive";

/** Skill status */
export type SkillStatus = "active" | "disabled" | "archived" | "deleted";

/** Skill entity */
export interface Skill {
  id: string;
  name: string;
  description: string;
  license?: string;
  compatibility?: string;
  metadata: Record<string, unknown>;
  allowed_tools?: string;
  source_type: SkillSourceType;
  status: SkillStatus;
  version: string;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
}

/** Skill content (full instructions + files) */
export interface SkillContent {
  skill_md: string;
  files: SkillFileEntry[];
}

/** File entry in a skill archive */
export interface SkillFileEntry {
  path: string;
  content: string;
}

/** Per-skill usage counts (active agents/harnesses referencing the skill capability). */
export interface SkillUsage {
  agents: number;
  harnesses: number;
}

/** Map keyed by SkillId (e.g. `skill_<32hex>`); skills with zero usage are absent. */
export type SkillUsageMap = Record<string, SkillUsage>;

/** Validation result for SKILL.md */
export interface SkillValidationResult {
  valid: boolean;
  name?: string;
  description?: string;
  errors: string[];
  warnings: string[];
}

/** Request to create a skill from SKILL.md */
export interface CreateSkillRequest {
  skill_md: string;
}

/** Request to update a skill */
export interface UpdateSkillRequest {
  skill_md?: string;
  status?: SkillStatus;
}

/** Request to validate SKILL.md */
export interface ValidateSkillRequest {
  skill_md: string;
}
