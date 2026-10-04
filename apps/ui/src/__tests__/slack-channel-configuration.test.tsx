jest.mock("@/hooks/use-health-issues", () => ({
  useHealthIssues: () => ({ data: { data: [], total: 0 }, isError: false }),
}));
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SlackChannelConfiguration } from "@/components/agents/integrations/slack-channel-configuration";
import { ChannelRow } from "@/components/agents/channels/channel-row";
import { useAgentChannels } from "@/hooks/use-agent-channels";
import {
  listAgentChannels,
  updateAgentChannel,
  getSlackInstallCapability,
  getSlackChannelManifest,
  listSlackWorkspaces,
} from "@/lib/api/agent-channels";
import type { AgentChannel, OpenApiAppChannel } from "@/lib/api/types";

jest.mock("@/providers/feature-flags-provider", () => ({ useFeatureFlag: () => false }));
jest.mock("@/lib/api/agent-channels", () => ({
  ...jest.requireActual("@/lib/api/agent-channels"),
  listAgentChannels: jest.fn(),
  updateAgentChannel: jest.fn(),
  getSlackInstallCapability: jest.fn(),
  getSlackChannelManifest: jest.fn(),
  listSlackWorkspaces: jest.fn(),
}));
let current: AgentChannel & Pick<OpenApiAppChannel, "agent_version_policy" | "agent_version_id">;
function Harness({ canManage = true }: { canManage?: boolean }) {
  const { channels } = useAgentChannels("agent_1");
  const [expanded, setExpanded] = useState(false);
  const channel = channels[0]?.channel;
  if (!channel) return null;
  return (
    <ChannelRow
      channel={channel}
      expanded={expanded}
      onToggle={() => setExpanded(!expanded)}
      configureHref={canManage ? "/agents/agent_1/channels/endpoint_1" : undefined}
      usePanel={
        <SlackChannelConfiguration agentId="agent_1" channel={channel} canManage={canManage} />
      }
    />
  );
}
function mount(canManage = true) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Harness canManage={canManage} />
    </QueryClientProvider>,
  );
}
async function expand() {
  fireEvent.click(await screen.findByRole("button", { name: /Expand .* details/ }));
}

describe("inline Slack channel configuration", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    current = {
      id: "endpoint_1",
      channel_type: "slack",
      channel_config: {
        session_strategy: "per_channel",
        reply_mode: "all_messages",
        team_id: "T1",
        signing_secret_configured: true,
        bot_token_configured: true,
        agent_surface_enabled: false,
        first_message_received_at: "2026-10-03T10:00:00Z",
      },
      enabled: true,
      status: "live",
      agent_version_policy: "pinned",
      agent_version_id: "version_1",
      created_at: "2026-10-03T00:00:00Z",
      updated_at: "2026-10-03T00:00:00Z",
    };
    (listAgentChannels as jest.Mock).mockImplementation(async () => [current]);
    (getSlackInstallCapability as jest.Mock).mockResolvedValue({
      supported: false,
      connected: false,
      reconnect_required: false,
      can_manage: true,
    });
    (getSlackChannelManifest as jest.Mock).mockResolvedValue({
      manifest_yaml: 'event_subscriptions:\n  request_url: "https://example.com/slack/events"',
      create_url: "https://api.slack.com/apps?new_app=1",
    });
    (updateAgentChannel as jest.Mock).mockImplementation(async (_agent, _id, request) => {
      current = {
        ...current,
        ...request,
        channel_config: { ...current.channel_config, ...request.channel_config },
        updated_at: "2026-10-03T11:00:00Z",
      };
      return current;
    });
  });
  it("shows an explicit expansion affordance and retains unsaved changes across collapse", async () => {
    mount();
    expect(await screen.findByText("Show configuration")).toBeVisible();
    expect(
      screen.queryByRole("form", { name: "Slack channel configuration" }),
    ).not.toBeInTheDocument();
    await expand();
    fireEvent.click(screen.getByRole("switch", { name: "Slack agent pane" }));
    expect(screen.getByText("Unsaved changes")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /Collapse .* details/ }));
    expect(
      screen.queryByRole("form", { name: "Slack channel configuration" }),
    ).not.toBeInTheDocument();
    await expand();
    expect(screen.getByRole("switch", { name: "Slack agent pane" })).toBeChecked();
    fireEvent.click(screen.getByRole("button", { name: "Discard changes" }));
    expect(screen.getByRole("switch", { name: "Slack agent pane" })).not.toBeChecked();
  });
  it("saves settings inline without replacing secrets or changing the pinned agent version", async () => {
    mount();
    await expand();
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
    fireEvent.click(screen.getByRole("switch", { name: "Slack agent pane" }));
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await screen.findByText("Changes saved");
    expect(current.channel_config).toEqual(
      expect.objectContaining({
        agent_surface_enabled: true,
        first_message_received_at: "2026-10-03T10:00:00Z",
      }),
    );
    const request = (updateAgentChannel as jest.Mock).mock.calls[0][2];
    expect(request.channel_config).not.toHaveProperty("signing_secret");
    expect(request.channel_config).not.toHaveProperty("bot_token");
    expect(request).not.toHaveProperty("agent_version_policy");
    expect(request).not.toHaveProperty("agent_version_id");
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "Slack agent pane" })).toBeChecked(),
    );
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
    expect(screen.getByText("Message received")).toBeVisible();
  });
  it("saves the response policy inline without feature enrollment", async () => {
    mount();
    await expand();
    fireEvent.click(screen.getByLabelText("Response policy"));
    const option = await screen.findByRole("option", { name: "Relevant messages" });
    fireEvent.pointerDown(option);
    fireEvent.click(option);
    expect(screen.getByText("Unsaved changes")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await screen.findByText("Changes saved");
    expect(current.channel_config).toMatchObject({ response_policy: "relevant_messages" });
    const request = (updateAgentChannel as jest.Mock).mock.calls[0][2];
    expect(request.channel_config.response_policy).toBe("relevant_messages");
    expect(request.channel_config).not.toHaveProperty("bot_token");
    expect(request).not.toHaveProperty("agent_version_id");
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
  });
  it("retains the draft and surfaces a failed save", async () => {
    (updateAgentChannel as jest.Mock).mockRejectedValue(new Error("Could not save channel"));
    mount();
    await expand();
    fireEvent.click(screen.getByRole("switch", { name: "Slack agent pane" }));
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save channel");
    expect(screen.getByRole("switch", { name: "Slack agent pane" })).toBeChecked();
    expect(screen.getByRole("button", { name: "Save changes" })).toBeEnabled();
  });
  it("loads the manifest only after opening manual configuration", async () => {
    mount();
    await expand();
    expect(screen.queryByLabelText("Signing secret")).not.toBeInTheDocument();
    expect(getSlackChannelManifest).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Configure manually" }));
    expect(screen.getByLabelText("Signing secret")).toBeVisible();
    await waitFor(() => expect(getSlackChannelManifest).toHaveBeenCalledWith("endpoint_1"));
  });
  it("renders connection evidence without settings controls for a viewer", async () => {
    mount(false);
    await expand();
    expect(screen.getByText("A signed Slack message has reached this channel.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Save changes" })).not.toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Slack agent pane" })).not.toBeInTheDocument();
    expect(getSlackInstallCapability).not.toHaveBeenCalled();
  });
  it("requires saving edits before installation and keeps the chosen workspace after saving", async () => {
    current = { ...current, channel_config: { session_strategy: "per_thread" }, status: "draft" };
    (getSlackInstallCapability as jest.Mock).mockResolvedValue({
      supported: true,
      connected: true,
      reconnect_required: false,
      can_manage: true,
    });
    (listSlackWorkspaces as jest.Mock).mockResolvedValue([
      { id: "workspace_1", team_id: "T1", team_name: "Acme", status: "connected" },
    ]);
    mount();
    await expand();
    await waitFor(() => expect(screen.getByRole("button", { name: "Add to Slack" })).toBeEnabled());
    fireEvent.click(screen.getByRole("switch", { name: "Slack agent pane" }));
    expect(screen.getByRole("button", { name: "Add to Slack" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await screen.findByText("Changes saved");
    await waitFor(() => expect(screen.getByRole("button", { name: "Add to Slack" })).toBeEnabled());
    expect(screen.getByText("Changes saved")).toBeVisible();
    expect(current.channel_config).toEqual(
      expect.objectContaining({ agent_surface_enabled: true }),
    );
  });

  it("keeps controls independent when more than one Slack form is mounted", () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={client}>
        <SlackChannelConfiguration agentId="agent_1" channel={current} canManage />
        <SlackChannelConfiguration
          agentId="agent_1"
          channel={{ ...current, id: "endpoint_2" }}
          canManage
        />
      </QueryClientProvider>,
    );
    const strategies = screen.getAllByRole("combobox", { name: "Session strategy" });
    expect(strategies[0].id).not.toBe(strategies[1].id);
    const panes = screen.getAllByRole("switch", { name: "Slack agent pane" });
    fireEvent.click(panes[0]);
    expect(panes[0]).toBeChecked();
    expect(panes[1]).not.toBeChecked();
  });

  it("does not show the old completed checklist or a separate Use it panel", async () => {
    mount();
    await expand();
    expect(screen.queryByText("Set up")).not.toBeInTheDocument();
    expect(screen.queryByText("Use it")).not.toBeInTheDocument();
    expect(screen.queryByText("1. Publish the channel")).not.toBeInTheDocument();
    expect(screen.queryByText(/Waiting for Slack/)).not.toBeInTheDocument();
  });
});
