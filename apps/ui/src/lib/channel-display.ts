import type {
  AgUiToolVisibility,
  AgentChannel,
  ChannelType,
  InvocationSessionMode,
  SessionStrategy,
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
