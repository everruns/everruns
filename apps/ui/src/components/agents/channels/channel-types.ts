import type { ChannelType } from "@/lib/api/types";

/// Who a channel type is for. The New channel page groups types by this
/// instead of listing protocols, because "who will use it" is the question a
/// person can answer without knowing what AG-UI or FCP is.
export type ChannelAudience = "people" | "software";

export interface ChannelTypeInfo {
  kind: ChannelType;
  audience: ChannelAudience;
  /** One plain sentence: what it is and who reaches the agent through it. */
  summary: string;
  /** What must exist before the channel can work, when anything must. */
  needs?: string;
}

/// Order is the order on the New channel page, most common first in each group.
/// Schedules are triggers and are never created here.
export const CHANNEL_TYPE_INFO: ChannelTypeInfo[] = [
  {
    kind: "work_chat",
    audience: "people",
    summary:
      "A dedicated chat site with its own sign-in: conversations, threads, files and approvals.",
  },
  {
    kind: "public_chat",
    audience: "people",
    summary:
      "A hosted chat page with your branding. Open to anyone with the link, or only to people who sign in.",
  },
  {
    kind: "slack",
    audience: "people",
    summary: "People mention or message the agent in your Slack workspace. It replies in Slack.",
    needs: "Needs a Slack workspace",
  },
  {
    kind: "voice",
    audience: "people",
    summary:
      "Talk to the agent out loud. A speech model listens and speaks; the agent writes every answer.",
    needs: "Needs an OpenAI provider",
  },
  {
    kind: "api",
    audience: "software",
    summary:
      "Call the agent from your own code with an agent key: start sessions, send messages, stream replies.",
  },
  {
    kind: "webhook",
    audience: "software",
    summary:
      "Another system sends an HTTP request with a token, and the agent runs with a message you set.",
  },
  {
    kind: "ag_ui",
    audience: "software",
    summary:
      "Put the agent inside your own web app using the open AG-UI protocol. Tool details stay hidden from visitors.",
  },
  {
    kind: "fcp",
    audience: "software",
    summary:
      "Text in, text out over HTTP. For simple clients and other agents that only send and read text.",
  },
];

export function channelTypeInfo(kind: ChannelType): ChannelTypeInfo | undefined {
  return CHANNEL_TYPE_INFO.find((info) => info.kind === kind);
}
