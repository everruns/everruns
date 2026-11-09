import type { SessionStrategy } from "./legacy-api-types";

export type SlackResponsePolicy = "all_messages" | "mentions_only" | "relevant_messages";

/** How Slack reaches the channel: an outgoing socket, or Slack calling a public URL. */
export type SlackConnection = "request_url" | "socket_mode";

export interface SlackChannelConfig {
  connection?: SlackConnection;
  app_token?: string;
  app_token_configured?: boolean;
  signing_secret?: string;
  signing_secret_configured?: boolean;
  bot_token?: string;
  bot_token_configured?: boolean;
  channel_id?: string;
  team_id?: string;
  session_strategy: SessionStrategy;
  response_policy?: SlackResponsePolicy;
  webhook_verified_at?: string | null;
  first_message_received_at?: string | null;
}
