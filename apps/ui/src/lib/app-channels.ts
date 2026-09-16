import type {
  AgUiToolVisibility,
  AppChannel,
  ChannelType,
  InvocationSessionMode,
  SessionStrategy,
  SlackReplyMode,
} from "@/lib/api/types";

export interface EndpointLifecyclePresentation {
  label: "live" | "disabled" | "draft";
  description: "Live" | "Paused" | "Draft — not accepting traffic";
  isLive: boolean;
}

export function getEndpointLifecyclePresentation(
  channel: Pick<AppChannel, "enabled" | "status">,
): EndpointLifecyclePresentation {
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
      return "API endpoint";
    case "public_chat":
      return "Public Chat";
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
      return "All Assistant Messages";
    case "report_progress_only":
      return "Report Progress Only";
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
