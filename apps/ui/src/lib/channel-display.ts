import type {
  AgUiToolVisibility,
  AgentChannel,
  ChannelType,
  InvocationSessionMode,
  SessionStrategy,
  SlackReplyMode,
} from "@/lib/api/types";

export interface ChannelLifecyclePresentation {
  label: "live" | "disabled" | "draft";
  description: "Live" | "Paused" | "Draft — not accepting traffic";
  isLive: boolean;
}

export function getChannelLifecyclePresentation(
  channel: Pick<AgentChannel, "enabled" | "status">,
): ChannelLifecyclePresentation {
  if (!channel.enabled) {
    return { label: "disabled", description: "Paused", isLive: false };
  }
  if (channel.status === "live") {
    return { label: "live", description: "Live", isLive: true };
  }
  return { label: "draft", description: "Draft — not accepting traffic", isLive: false };
}

/**
 * Publish and disable are different controls.
 *
 * Publish and unpublish set status to live or draft and leave the channel
 * enabled, so unpublish closes it and publish opens it again. Disable
 * (`enabled: false`) writes status `disabled`. Turning it back on writes
 * draft, not live, so the channel stays closed until a separate publish.
 */
export function channelPublishControl(channel: Pick<AgentChannel, "enabled" | "status">): {
  label: "Off" | "Published" | "Draft";
  hint: string;
  live: boolean;
  /** False while disabled: enable (back to draft) and save before publishing. */
  canPublish: boolean;
} {
  if (!channel.enabled) {
    return {
      label: "Off",
      hint: "This channel is off. Enable it and save, then publish. Enabling returns a draft.",
      live: false,
      canPublish: false,
    };
  }
  if (getChannelLifecyclePresentation(channel).isLive) {
    return {
      label: "Published",
      hint: "Unpublish closes this channel and leaves it ready to publish again.",
      live: true,
      canPublish: true,
    };
  }
  return {
    label: "Draft",
    hint: "Publish opens this channel to callers.",
    live: false,
    canPublish: true,
  };
}

export const CHANNEL_DISABLE_HINT =
  "Off stops traffic. Turning it back on leaves a draft, which stays closed until you publish.";

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
