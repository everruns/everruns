// App types (deployable agent+harness bundles)

export type AppStatus = "draft" | "published" | "archived" | "deleted";
export type ChannelType = "slack";
export type SessionStrategy = "per_thread" | "per_channel" | "per_user";
export type SlackReplyMode = "all_messages" | "report_progress_only";

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

export interface App {
  id: string;
  name: string;
  description: string | null;
  harness_id: string;
  agent_id: string;
  agent_identity_id?: string | null;
  channel_type: ChannelType;
  channel_config: SlackChannelConfig | Record<string, unknown>;
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
  agent_id: string;
  agent_identity_id?: string;
  channel_type: ChannelType;
  channel_config?: SlackChannelConfig | Record<string, unknown>;
}

export interface UpdateAppRequest {
  name?: string;
  description?: string;
  harness_id?: string;
  agent_id?: string;
  agent_identity_id?: string | null;
  channel_type?: ChannelType;
  channel_config?: SlackChannelConfig | Record<string, unknown>;
  status?: AppStatus;
}
