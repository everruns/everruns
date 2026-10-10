import { render, screen } from "@testing-library/react";
import { AgentCardChannels } from "@/components/agents/agent-card-channels";
import type { Agent, AgentChannelSummary } from "@/lib/api/types";

const agent: Agent = {
  id: "agent_test",
  name: "test",
  display_name: null,
  description: null,
  system_prompt: "",
  harness_id: "harness_generic",
  default_model_id: null,
  tags: [],
  capabilities: [],
  status: "active",
  created_at: "2026-10-03T00:00:00Z",
  updated_at: "2026-10-03T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};
const channel = (
  id: string,
  channel_type: AgentChannelSummary["channel_type"],
  status: AgentChannelSummary["status"] = "live",
  enabled = true,
): AgentChannelSummary => ({ id, channel_type, status, enabled });

it("names channels and their states accessibly and links to integrations", () => {
  render(
    <AgentCardChannels
      agent={{
        ...agent,
        channels: [
          channel("slack", "slack"),
          channel("hook", "webhook", "draft"),
          channel("chat", "public_chat", "disabled", false),
        ],
      }}
    />,
  );
  expect(screen.getByRole("link", { name: "Slack: Live" })).toHaveAttribute(
    "href",
    "/agents/agent_test?tab=integrations",
  );
  expect(
    screen.getByRole("link", { name: "Webhook: Draft — not accepting traffic" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("link", { name: "Public chat: Draft — not accepting traffic" }),
  ).toBeInTheDocument();
});

it("does not advertise suspended or archived agents as live", () => {
  const channels = [channel("slack", "slack")];
  const { rerender } = render(
    <AgentCardChannels agent={{ ...agent, channels, exposures_suspended: true }} />,
  );
  expect(screen.getByRole("link", { name: "Slack: Suspended" })).toBeInTheDocument();
  rerender(<AgentCardChannels agent={{ ...agent, channels, status: "archived" }} />);
  expect(screen.getByRole("link", { name: "Slack: Agent unavailable" })).toBeInTheDocument();
});

it("keeps triggers out of channels and distinguishes empty from unavailable data", () => {
  const { rerender } = render(<AgentCardChannels agent={agent} canManage />);
  expect(screen.getByText("Not loaded")).toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Add" })).not.toBeInTheDocument();
  rerender(
    <AgentCardChannels
      agent={{ ...agent, channels: [channel("schedule", "schedule")] }}
      canManage
    />,
  );
  expect(screen.getByText("None configured")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Add" })).toHaveAttribute(
    "href",
    "/agents/agent_test/channels/new",
  );
  rerender(<AgentCardChannels agent={{ ...agent, channels: [] }} />);
  expect(screen.queryByRole("link", { name: "Add" })).not.toBeInTheDocument();
});

it("provides accurate overflow counts for each container width", () => {
  render(
    <AgentCardChannels
      agent={{
        ...agent,
        channels: [
          channel("slack", "slack"),
          channel("chat", "public_chat"),
          channel("hook", "webhook"),
          channel("a2a", "a2a"),
          channel("api", "api_endpoint"),
        ],
      }}
    />,
  );
  expect(
    screen.getAllByRole("link", { name: "View all 5 channels" }).map((link) => link.textContent),
  ).toEqual(["+4", "+3", "+2"]);
  expect(screen.queryByText("A2A")).not.toBeInTheDocument();
});
