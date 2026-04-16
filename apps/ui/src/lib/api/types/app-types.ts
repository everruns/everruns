// App types (deployable agent+harness bundles with multi-channel support)

export type AppStatus = "draft" | "published" | "archived" | "deleted";
export type ChannelType = "slack" | "ag_ui";
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

export interface AgUiChannelConfig {
  anonymous?: boolean;
}

export interface AppChannel {
  id: string;
  channel_type: ChannelType;
  channel_config: SlackChannelConfig | AgUiChannelConfig | Record<string, unknown>;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface App {
  id: string;
  name: string;
  description: string | null;
  harness_id: string;
  agent_id: string;
  agent_identity_id?: string | null;
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
  agent_id: string;
  agent_identity_id?: string;
  channel_type: ChannelType;
  channel_config?: SlackChannelConfig | AgUiChannelConfig | Record<string, unknown>;
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
  channel_config?: SlackChannelConfig | AgUiChannelConfig | Record<string, unknown>;
  enabled?: boolean;
}

export interface UpdateChannelRequest {
  channel_type?: ChannelType;
  channel_config?: SlackChannelConfig | AgUiChannelConfig | Record<string, unknown>;
  enabled?: boolean;
}
