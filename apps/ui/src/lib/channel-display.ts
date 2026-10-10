import type {
  AgUiToolVisibility,
  AgentChannel,
  ChannelType,
  InvocationSessionMode,
  SessionStrategy,
  SlackReplyMode,
} from "@/lib/api/types";

export interface ChannelLifecyclePresentation {
  label: "live" | "draft";
  description: "Live" | "Draft — not accepting traffic";
  isLive: boolean;
}

export function getChannelLifecyclePresentation(
  channel: Pick<AgentChannel, "enabled" | "status">,
): ChannelLifecyclePresentation {
  // Open or closed. A stored `disabled` flag is closed, same as a draft:
  // publish is what opens it. Pausing the agent is a separate, outer switch.
  if (channel.enabled && channel.status === "live") {
    return { label: "live", description: "Live", isLive: true };
  }
  return { label: "draft", description: "Draft — not accepting traffic", isLive: false };
}

/**
 * One control. Publish opens the channel to callers. Unpublish closes it.
 * Publish again opens it, including a channel stored as disabled.
 */
export function channelPublishControl(channel: Pick<AgentChannel, "enabled" | "status">): {
  label: "Published" | "Draft";
  hint: string;
  live: boolean;
} {
  if (getChannelLifecyclePresentation(channel).isLive) {
    return {
      label: "Published",
      hint: "Unpublish closes this channel. Publish again to open it.",
      live: true,
    };
  }
  return {
    label: "Draft",
    hint: "Publish opens this channel to callers.",
    live: false,
  };
}

export function getChannelTypeDisplayName(channelType: ChannelType): string {
  switch (channelType) {
    case "ag_ui":
      return "AG-UI";
    case "schedule":
      return "Schedule";
    case "slack":
      return "Slack";
    case "webhook":
      return "Webhook";
    case "a2a":
      return "A2A (Agent2Agent)";
    case "fcp":
      return "FCP (Free Communication Protocol)";
    case "api_endpoint":
      return "API channel";
    case "api":
      return "Agent API";
    case "public_chat":
      return "Public Chat";
    case "voice":
      return "Voice";
  }
}

export function getInvocationSessionModeDisplayName(mode: InvocationSessionMode): string {
  switch (mode) {
    case "session_per_invocation":
      return "Session Per Invocation";
    case "shared_session":
      return "Shared Session";
  }
}

export function getSessionStrategyDisplayName(strategy: SessionStrategy): string {
  switch (strategy) {
    case "per_thread":
      return "Per Thread";
    case "per_channel":
      return "Per Channel";
    case "per_user":
      return "Per User";
  }
}

export function getSlackReplyModeDisplayName(mode: SlackReplyMode): string {
  switch (mode) {
    case "all_messages":
      return "Automatic replies";
    case "tool_only":
      return "Agent-controlled messages";
  }
}

export function getAgUiToolVisibilityDisplayName(visibility: AgUiToolVisibility): string {
  switch (visibility) {
    case "none":
      return "None";
    case "generic":
      return "Generic";
    case "narrated":
      return "Narrated";
  }
}
