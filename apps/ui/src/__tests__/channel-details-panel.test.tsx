import { render, screen } from "@testing-library/react";
import { ChannelDetailsPanel } from "@/components/agents/integrations/channel-details-panel";
import type { AgentChannel } from "@/lib/api/types";

const mockPush = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush }),
}));

jest.mock("@/components/agents/integrations/slack-channel-configuration", () => ({
  SlackChannelConfiguration: ({ canManage }: { canManage: boolean }) => (
    <div data-testid="slack-configuration" data-can-manage={String(canManage)} />
  ),
}));

function channel(overrides: Partial<AgentChannel>): AgentChannel {
  return {
    id: "appchan_123",
    channel_type: "slack",
    channel_config: {
      session_strategy: "per_thread",
    },
    enabled: true,
    status: "live",
    created_at: "2026-09-19T00:00:00Z",
    updated_at: "2026-09-19T00:00:00Z",
    ...overrides,
  };
}

describe("ChannelDetailsPanel", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(window, "open").mockImplementation(() => null);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("renders the combined Slack configuration instead of setup and use panels", () => {
    render(
      <ChannelDetailsPanel
        agentId="agent_123"
        agentName="Support Agent"
        channel={channel({})}
        configureHref="/agents/agent_123/channels/appchan_123"
      />,
    );
    expect(screen.getByTestId("slack-configuration")).toHaveAttribute("data-can-manage", "true");
    expect(screen.queryByText("Set up")).not.toBeInTheDocument();
    expect(screen.queryByText("Use it")).not.toBeInTheDocument();
  });
  it("renders Slack details without editing access", () => {
    render(
      <ChannelDetailsPanel agentId="agent_123" agentName="Support Agent" channel={channel({})} />,
    );
    expect(screen.getByTestId("slack-configuration")).toHaveAttribute("data-can-manage", "false");
  });

  it("mounts the A2A Agent Card setup path in the expanded channel details", () => {
    render(
      <ChannelDetailsPanel
        agentId="agent_123"
        agentName="Support Agent"
        agentDescription="Answers support questions"
        channel={channel({
          channel_type: "a2a",
          status: "draft",
          channel_config: {
            api_key_prefix: "evra2a_12345678...",
            session_mode: "shared_session",
            message: "Handle {{a2a.text}}",
          },
        })}
      />,
    );

    expect(screen.getByText("Agent Card")).toBeInTheDocument();
    expect(screen.getByText(/Publish and enable this channel/)).toBeInTheDocument();
    expect(screen.getByText("Use it")).toBeInTheDocument();
  });

  it.each([
    [
      "AG-UI",
      channel({
        channel_type: "ag_ui",
        channel_config: { anonymous: true, session_expiration_seconds: 3600 },
      }),
      "Image upload",
    ],
    [
      "FCP",
      channel({
        channel_type: "fcp",
        channel_config: { anonymous: true, session_expiration_seconds: 3600 },
      }),
      "Handshake (GET body)",
    ],
  ])("mounts %s setup guidance", (_name, channel, expectedText) => {
    render(
      <ChannelDetailsPanel agentId="agent_123" agentName="Support Agent" channel={channel} />,
    );

    expect(screen.getByText(expectedText)).toBeInTheDocument();
    expect(screen.getByText("Use it")).toBeInTheDocument();
  });
});
