import type { SessionStrategy } from "./legacy-api-types";

export type SlackResponsePolicy = "all_messages" | "mentions_only" | "relevant_messages";

export interface SlackChannelConfig {
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
