import { hasSlackCredentials } from "@/components/agents/integrations/slack-setup-guidance";
import type {
  AgentChannel,
  PublicChatChannelConfig,
  ScheduleChannelConfig,
  SlackChannelConfig,
  VoiceChannelConfig,
  WebhookChannelConfig,
} from "@/lib/api/types";
import { getChannelLifecyclePresentation, getChannelTypeDisplayName } from "@/lib/channel-display";

/// Plain facts about one channel, shared by the channel list and the channel
/// page so both say the same thing.

/** The name people know the channel by: its own name when it has one, else its type. */
export function channelDisplayName(channel: AgentChannel): string {
  if (channel.channel_type === "schedule") {
    const config = channel.channel_config as ScheduleChannelConfig;
    return config.message?.trim() || "Scheduled invocation";
  }
  if (channel.channel_type === "public_chat") {
    const config = channel.channel_config as PublicChatChannelConfig;
    return config.branding?.display_name?.trim() || "Public Chat";
  }
  if (channel.channel_type === "slack") {
    const config = channel.channel_config as SlackChannelConfig;
    return config.channel_id ? `Slack · ${config.channel_id}` : "Slack";
  }
  return getChannelTypeDisplayName(channel.channel_type);
}

/** Who can reach a Public Chat, in words. */
export function publicChatAccess(channel: AgentChannel): string {
  const config = channel.channel_config as PublicChatChannelConfig;
  const auth = channel.auth ?? config.auth;
  if (auth && auth.mode !== "anonymous") return "Only people who sign in";
  if (config.anonymous === false) return "Nobody yet: turn on a sign-in";
  if (config.token_configured || config.token) return "People with the link and token";
  return "Anyone with the link";
}

/** One short line of what matters about this channel's setup. */
export function channelFacts(channel: AgentChannel): string[] {
  const facts = [getChannelTypeDisplayName(channel.channel_type)];
  switch (channel.channel_type) {
    case "public_chat": {
      const config = channel.channel_config as PublicChatChannelConfig;
      facts.push(publicChatAccess(channel));
      if (config.captcha?.enabled) facts.push("Bot check on");
      break;
    }
    case "work_chat": {
      const config = channel.channel_config as { interface_mode?: string };
      facts.push(
        config.interface_mode === "single_with_threads"
          ? "Single chat with threads"
          : "Multiple conversations",
      );
      break;
    }
    case "webhook": {
      const config = channel.channel_config as WebhookChannelConfig;
      facts.push(config.token_configured || config.token ? "Token set" : "No token set");
      break;
    }
    case "slack": {
      const config = channel.channel_config as SlackChannelConfig;
      facts.push(
        hasSlackCredentials(config)
          ? config.team_id
            ? `Workspace ${config.team_id}`
            : "Connected"
          : "Not installed in a workspace yet",
      );
      break;
    }
    case "api":
      facts.push("Called with an agent key");
      break;
    case "ag_ui":
      facts.push("Embedded in your web app");
      break;
    case "fcp":
      facts.push("Text in, text out over HTTP");
      break;
    case "voice": {
      const config = channel.channel_config as VoiceChannelConfig;
      facts.push(`Voice ${config.voice ?? "marin"}`);
      break;
    }
  }
  return facts;
}

export type ChannelStatus = "published" | "draft" | "needs_setup";

/// Published and Draft come from the channel itself. Needs setup marks a
/// channel that cannot work yet even when published, so the list can point at
/// it: today that is a Slack channel with no app installed.
export function channelStatus(channel: AgentChannel): ChannelStatus {
  if (
    channel.channel_type === "slack" &&
    !hasSlackCredentials(channel.channel_config as SlackChannelConfig)
  ) {
    return "needs_setup";
  }
  return getChannelLifecyclePresentation(channel).isLive ? "published" : "draft";
}

export const CHANNEL_STATUS_LABEL: Record<ChannelStatus, string> = {
  published: "Published",
  draft: "Draft",
  needs_setup: "Needs setup",
};

/** The hosted page a Public Chat serves, on this deployment's own origin. */
export function publicChatUrl(channelId: string): string {
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  return `${origin}/public-chat/${channelId}`;
}

/** "2 hours ago", or "Never" for a channel nobody has used. */
export function lastUsedLabel(value?: string | null): string {
  if (!value) return "Never";
  const seconds = Math.round((new Date(value).getTime() - Date.now()) / 1000);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 60) return formatter.format(seconds, "second");
  if (abs < 3600) return formatter.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return formatter.format(Math.round(seconds / 3600), "hour");
  return formatter.format(Math.round(seconds / 86400), "day");
}
