// App types (deployable agent+harness bundles with multi-channel support)

import type { PrincipalSummary } from "./common-types";

export type AppStatus = "draft" | "published" | "archived" | "deleted";
export type ChannelType = "slack" | "ag_ui" | "schedule" | "webhook";
export type SessionStrategy = "per_thread" | "per_channel" | "per_user";
export type SlackReplyMode = "all_messages" | "report_progress_only";
export type InvocationSessionMode = "shared_session" | "session_per_invocation";
export type AgUiToolVisibility = "none" | "generic" | "narrated";

export interface SlackChannelConfig {
  signing_secret: string;
  bot_token: string;
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
}

export interface ScheduleChannelConfig {
  cron_expression: string;
  timezone?: string;
  session_mode?: InvocationSessionMode;
  message: string;
}

export interface WebhookChannelConfig {
  token: string;
  session_mode?: InvocationSessionMode;
  message: string;
}

export interface AppChannel {
  id: string;
  channel_type: ChannelType;
  channel_config:
    | SlackChannelConfig
    | AgUiChannelConfig
    | ScheduleChannelConfig
    | WebhookChannelConfig
    | Record<string, unknown>;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface App {
  id: string;
  name: string;
  description: string | null;
  harness_id: string;
  agent_id: string | null;
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
  agent_identity_id?: string;
  channel_type?: ChannelType;
  channel_config?:
    | SlackChannelConfig
    | AgUiChannelConfig
    | ScheduleChannelConfig
    | WebhookChannelConfig
    | Record<string, unknown>;
}

export interface UpdateAppRequest {
  name?: string;
  description?: string;
  harness_id?: string;
  agent_id?: string;
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
    | Record<string, unknown>;
  enabled?: boolean;
}
