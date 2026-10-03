import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  SlackConnectionStatus,
  SlackManualSetup,
} from "@/components/agents/integrations/slack-setup-guidance";
import { getSlackChannelManifest } from "@/lib/api/agent-channels";
import type { AgentChannel, SlackChannelConfig } from "@/lib/api/types";

jest.mock("@/lib/api/agent-channels", () => ({ getSlackChannelManifest: jest.fn() }));
const configured = {
  session_strategy: "per_thread",
  signing_secret_configured: true,
  bot_token_configured: true,
} satisfies SlackChannelConfig;
function channel(
  config: SlackChannelConfig = configured,
  status: AgentChannel["status"] = "live",
): AgentChannel {
  return {
    id: "endpoint_1",
    channel_type: "slack",
    channel_config: config,
    enabled: true,
    status,
    created_at: "2026-10-03T00:00:00Z",
    updated_at: "2026-10-03T00:00:00Z",
  };
}
function renderManual(value: AgentChannel) {
  return render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <SlackManualSetup channel={value} />
    </QueryClientProvider>,
  );
}

describe("Slack connection guidance", () => {
  beforeEach(() => jest.clearAllMocks());
  it("removes completed setup steps and accepts signed messages without a URL challenge", () => {
    render(
      <SlackConnectionStatus
        channel={channel({ ...configured, first_message_received_at: "2026-10-03T10:00:00Z" })}
      />,
    );
    expect(screen.getByText("Message received")).toBeInTheDocument();
    expect(screen.getByText(/A signed Slack message/)).toBeInTheDocument();
    expect(
      screen.queryByText(/Publish the channel|Waiting for Slack|Invite the bot/),
    ).not.toBeInTheDocument();
  });
  it("does not claim credentials alone establish a verified connection", () => {
    render(<SlackConnectionStatus channel={channel()} />);
    expect(screen.getByText("Credentials saved")).toBeInTheDocument();
    expect(screen.getByText(/No Slack message received yet/)).toBeInTheDocument();
  });
  it("shows the remaining test action after URL verification", () => {
    render(
      <SlackConnectionStatus
        channel={channel({ ...configured, webhook_verified_at: "2026-10-03T10:00:00Z" })}
      />,
    );
    expect(screen.getByText("Request URL verified")).toBeInTheDocument();
    expect(screen.getByText(/@mention it to test/)).toBeInTheDocument();
  });
  it("keeps publication separate from observed delivery", () => {
    render(
      <SlackConnectionStatus
        channel={channel(
          { ...configured, first_message_received_at: "2026-10-03T10:00:00Z" },
          "draft",
        )}
      />,
    );
    expect(screen.getByText("Message received")).toBeInTheDocument();
    expect(screen.getByText(/Publish this channel/)).toBeInTheDocument();
  });
  it("does not consider partial credentials configured", () => {
    render(
      <SlackConnectionStatus channel={channel({ ...configured, bot_token_configured: false })} />,
    );
    expect(screen.getByText("Not connected")).toBeInTheDocument();
  });
  it("keeps manual app creation behind publication", () => {
    renderManual(channel({ session_strategy: "per_thread" }, "draft"));
    expect(screen.getByText(/Publish this channel before creating/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Create Slack app" })).not.toBeInTheDocument();
    expect(getSlackChannelManifest).not.toHaveBeenCalled();
  });
  it("opens the channel manifest only with a public HTTPS request URL", async () => {
    const open = jest.spyOn(window, "open").mockImplementation(() => null);
    (getSlackChannelManifest as jest.Mock).mockResolvedValue({
      manifest_yaml: 'event_subscriptions:\n  request_url: "https://example.com/slack/events"',
      create_url: "https://api.slack.com/apps?new_app=1",
    });
    renderManual(channel({ session_strategy: "per_thread" }));
    const create = screen.getByRole("button", { name: "Create Slack app" });
    await waitFor(() => expect(create).toBeEnabled());
    fireEvent.click(create);
    expect(open).toHaveBeenCalledWith(
      "https://api.slack.com/apps?new_app=1",
      "_blank",
      "noopener,noreferrer",
    );
    open.mockRestore();
  });
  it("disables manual app creation on localhost", async () => {
    (getSlackChannelManifest as jest.Mock).mockResolvedValue({
      manifest_yaml: 'event_subscriptions:\n  request_url: "http://localhost:9300/slack/events"',
      create_url: "https://api.slack.com/apps?new_app=1",
    });
    renderManual(channel({ session_strategy: "per_thread" }));
    await screen.findByText(/PUBLIC_APP_URL/);
    expect(screen.getByRole("button", { name: "Create Slack app" })).toBeDisabled();
  });
  it("shows a retry action when loading a manual manifest fails", async () => {
    (getSlackChannelManifest as jest.Mock).mockRejectedValue(new Error("unavailable"));
    renderManual(channel());
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not load the Slack app manifest",
    );
    expect(screen.getByRole("button", { name: "Try again" })).toBeEnabled();
  });
});
