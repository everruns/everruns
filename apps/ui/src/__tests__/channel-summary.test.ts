import {
  channelDisplayName,
  channelFacts,
  channelStatus,
  publicChatAccess,
} from "@/components/agents/channels/channel-summary";
import type { AgentChannel } from "@/lib/api/types";

function channel(overrides: Partial<AgentChannel>): AgentChannel {
  return {
    id: "appchan_1",
    channel_type: "api",
    channel_config: {},
    enabled: true,
    status: "draft",
    created_at: "2026-10-29T00:00:00Z",
    updated_at: "2026-10-29T00:00:00Z",
    ...overrides,
  };
}

describe("channel summary", () => {
  it("names a channel by its own name, else its type", () => {
    expect(
      channelDisplayName(
        channel({
          channel_type: "public_chat",
          channel_config: { branding: { display_name: "Help desk" } },
        }),
      ),
    ).toBe("Help desk");
    expect(channelDisplayName(channel({ channel_type: "public_chat" }))).toBe("Public Chat");
    expect(channelDisplayName(channel({}))).toBe("Agent API");
  });

  it("calls a channel published only when it is enabled and live", () => {
    expect(channelStatus(channel({ status: "live" }))).toBe("published");
    expect(channelStatus(channel({ status: "draft" }))).toBe("draft");
    expect(channelStatus(channel({ status: "live", enabled: false }))).toBe("draft");
  });

  it("flags a Slack channel with no app installed as needing setup, even when published", () => {
    expect(channelStatus(channel({ channel_type: "slack", status: "live" }))).toBe("needs_setup");
  });

  it("says who can reach a Public Chat", () => {
    expect(publicChatAccess(channel({ channel_type: "public_chat" }))).toBe("Anyone with the link");
    expect(
      publicChatAccess(
        channel({ channel_type: "public_chat", channel_config: { token_configured: true } }),
      ),
    ).toBe("People with the link and token");
    expect(
      publicChatAccess(
        channel({
          channel_type: "public_chat",
          channel_config: { auth: { mode: "required" } },
        } as Partial<AgentChannel>),
      ),
    ).toBe("Only people who sign in");
  });

  it("leads the facts with the type, then what matters about the setup", () => {
    expect(channelFacts(channel({}))).toEqual(["Agent API", "Called with an agent key"]);
    expect(
      channelFacts(
        channel({ channel_type: "public_chat", channel_config: { captcha: { enabled: true } } }),
      ),
    ).toEqual(["Public Chat", "Anyone with the link", "Bot check on"]);
  });
});
