import { render, screen } from "@testing-library/react";
import { AgentRow } from "@/components/agents/home/agent-row";
import { ChannelRow } from "@/components/agents/home/channel-row";
import { ChannelRow as IntegrationsChannelRow } from "@/components/agents/channels/channel-row";
import type { OrgExposure } from "@/hooks/use-org-exposures";
import type { Agent, AgentChannel } from "@/lib/api/types";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href, ...props }: React.ComponentPropsWithoutRef<"a">) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

jest.mock("@/hooks/use-agent-channels", () => ({
  usePublishAgentChannel: () => ({ mutate: jest.fn(), isPending: false }),
}));

function dadJokes(): Agent {
  return {
    id: "agent_dj",
    name: "dad-jokes",
    display_name: "Dad Jokes",
    status: "active",
    exposures_suspended: false,
    channels: [
      {
        id: "appchan_slack",
        channel_type: "slack",
        enabled: true,
        status: "live",
      },
    ],
  } as Agent;
}

describe("agent and channel links", () => {
  it("links each channel on an agent to that channel", () => {
    render(<AgentRow agent={dadJokes()} activity={undefined} attention={[]} />);

    expect(screen.getByRole("link", { name: "Slack channel, Live" })).toHaveAttribute(
      "href",
      "/agents/agent_dj?tab=integrations&channel=appchan_slack",
    );
    expect(screen.getByRole("link", { name: "Dad Jokes" })).toHaveAttribute(
      "href",
      "/agents/agent_dj",
    );
  });

  it("links a channel row to that channel on its agent", () => {
    const exposure = {
      channel: {
        id: "appchan_slack",
        channel_type: "slack",
        channel_config: {},
        enabled: true,
        status: "draft",
      },
      agent: dadJokes(),
      state: "draft",
      anonymous: false,
      publiclyReachable: false,
      isTrigger: false,
      lastInvokedAt: null,
    } as OrgExposure;

    render(<ChannelRow exposure={exposure} activity={undefined} canManage />);

    expect(screen.getByRole("link", { name: "Slack" })).toHaveAttribute(
      "href",
      "/agents/agent_dj?tab=integrations&channel=appchan_slack",
    );
    expect(screen.getByRole("link", { name: /Dad Jokes/ })).toHaveAttribute(
      "href",
      "/agents/agent_dj?tab=integrations",
    );
    expect(screen.getByRole("button", { name: "Publish" })).toHaveAttribute(
      "title",
      "Publish opens this channel to callers.",
    );
  });
});

describe("channel publish switch", () => {
  it("publishes a disabled channel in one step", () => {
    render(
      <IntegrationsChannelRow
        channel={
          {
            id: "appchan_slack",
            channel_type: "webhook",
            channel_config: {},
            enabled: false,
            status: "disabled",
            created_at: "2026-10-01T00:00:00Z",
            updated_at: "2026-10-01T00:00:00Z",
          } as AgentChannel
        }
        expanded={false}
        onToggle={() => undefined}
        onPublishChange={() => undefined}
      />,
    );

    expect(screen.queryByRole("switch", { name: "Enabled" })).not.toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /Publish Webhook channel. Publish opens this channel/ }),
    ).toBeEnabled();
  });
});
