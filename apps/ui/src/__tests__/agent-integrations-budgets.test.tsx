import { render, screen } from "@testing-library/react";
import { AgentIntegrationsPanel } from "@/components/agents/agent-integrations-panel";
import type { Agent, AgentChannel } from "@/lib/api/types";

const mockUseFeatureFlag = jest.fn();
const mockCanAgent = jest.fn();
const mockCanBudget = jest.fn();
const mockReplace = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({ replace: mockReplace, push: jest.fn() }),
}));

jest.mock("@/hooks/use-agent-source", () => ({
  useAgentSource: () => ({ data: null, isLoading: false, isError: false }),
}));
jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: (flag: string) => mockUseFeatureFlag(flag),
}));

jest.mock("@/hooks/use-agent-channels", () => ({
  isTriggerChannel: () => false,
  useAgentChannels: () => ({
    channels: [
      {
        channel: {
          id: "endpoint_1",
          channel_type: "webhook",
          channel_config: {},
          enabled: true,
          status: "live",
          created_at: "2026-09-19T00:00:00Z",
          updated_at: "2026-09-19T00:00:00Z",
        } satisfies AgentChannel,
      },
    ],
    isLoading: false,
  }),
  usePublishAgentChannel: () => ({ mutate: jest.fn(), isPending: false }),
  useTriggerAgentChannel: () => ({ mutate: jest.fn(), isPending: false }),
}));

jest.mock("@/hooks/use-agent-triggers", () => ({
  useAgentTriggers: () => ({ data: [] }),
  useUpdateAgentTrigger: () => ({ mutate: jest.fn(), isPending: false }),
  useDeleteAgentTrigger: () => ({ mutate: jest.fn() }),
  useRunAgentTrigger: () => ({ mutate: jest.fn(), isPending: false }),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: (resource: string) => ({
    can: resource === "budgets" ? mockCanBudget : mockCanAgent,
  }),
}));

jest.mock("@/hooks/use-agents", () => ({
  useResumeAgentExposures: () => ({ mutate: jest.fn(), isPending: false }),
  useSuspendAgentExposures: () => ({ mutate: jest.fn(), isPending: false }),
}));

jest.mock("@/components/agents/channels/channel-row", () => ({
  ChannelRow: ({
    usePanel,
    expanded,
    channel,
  }: {
    usePanel?: React.ReactNode;
    expanded?: boolean;
    channel: { id: string };
  }) => (
    <div data-testid={`row-${channel.id}`} data-expanded={String(!!expanded)}>
      {usePanel}
    </div>
  ),
}));

jest.mock("@/components/agents/agent-github-card", () => ({
  AgentGitHubSection: ({ githubTriggerCount }: { githubTriggerCount?: number }) => (
    <section data-testid="github-section" data-github-triggers={String(githubTriggerCount)}>
      <h3>GitHub</h3>
    </section>
  ),
}));

jest.mock("@/components/budgets/budget-panel", () => ({
  BudgetPanel: ({
    subjectType,
    subjectId,
    canManage,
  }: {
    subjectType: string;
    subjectId: string;
    canManage: boolean;
  }) => (
    <div data-testid={`budget-${subjectType}-${subjectId}`} data-can-manage={String(canManage)} />
  ),
}));

const agent = {
  id: "agent_1",
  name: "test-agent",
  display_name: "Test Agent",
  description: null,
  system_prompt: "",
  harness_id: "harness_1",
  default_model_id: null,
  tags: [],
  capabilities: [],
  status: "active",
  created_at: "2026-09-19T00:00:00Z",
  updated_at: "2026-09-19T00:00:00Z",
  archived_at: null,
  deleted_at: null,
} as Agent;

describe("AgentIntegrationsPanel budgets", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseFeatureFlag.mockReturnValue(true);
    mockCanAgent.mockReturnValue(true);
  });

  it.each([
    { role: "Member", canManage: false },
    { role: "Admin", canManage: true },
    { role: "Owner", canManage: true },
  ])("gives $role the agent budget controls allowed by policy", ({ canManage }) => {
    mockCanBudget.mockImplementation((policy: string) =>
      policy === "budget.view" ? true : canManage,
    );

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.getByTestId("budget-agent-agent_1")).toHaveAttribute(
      "data-can-manage",
      String(canManage),
    );
    // A channel's own budget lives on its channel page, not in the list.
    expect(screen.queryByTestId("budget-agent_channel-endpoint_1")).not.toBeInTheDocument();
  });

  it("lists each channel as a row that opens its page", () => {
    mockCanBudget.mockReturnValue(false);

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.getByText(/1 channel, 1 published/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Webhook" })).toHaveAttribute(
      "href",
      "/agents/agent_1/channels/endpoint_1",
    );
    expect(screen.getAllByText("Published").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Never").length).toBeGreaterThan(0);
  });

  it("does not fetch or mount budgets without budget.view", () => {
    mockCanBudget.mockReturnValue(false);

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.queryByTestId(/^budget-/)).not.toBeInTheDocument();
  });

  it("enables or disables every channel and keeps each channel's publish state", () => {
    mockCanBudget.mockReturnValue(false);

    const { rerender } = render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.getByRole("switch", { name: "Accept traffic" })).toBeChecked();
    expect(
      screen.getByText(
        "Off pauses every channel at once. Each channel keeps its own Published or Draft setting.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("All channels are paused")).not.toBeInTheDocument();

    rerender(<AgentIntegrationsPanel agent={{ ...agent, exposures_suspended: true }} />);

    expect(screen.getByRole("switch", { name: "Accept traffic" })).not.toBeChecked();
    expect(screen.getByText("All channels are paused")).toBeInTheDocument();
  });

  it("sends a link to a channel on to that channel's page", () => {
    mockCanBudget.mockReturnValue(false);

    render(<AgentIntegrationsPanel agent={agent} initialChannelId="endpoint_1" />);

    expect(mockReplace).toHaveBeenCalledWith("/agents/agent_1/channels/endpoint_1");
  });

  it("gives GitHub its own section between Channels and Triggers", () => {
    mockUseFeatureFlag.mockReturnValue(false);
    mockCanBudget.mockReturnValue(false);

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.getAllByText("Triggers")).toHaveLength(1);
    expect(screen.getByRole("link", { name: "Add trigger" })).toHaveAttribute(
      "href",
      "/agents/agent_1/triggers/new",
    );
    expect(screen.getByRole("heading", { name: "No triggers yet" })).toBeInTheDocument();

    const headings = screen
      .getAllByRole("heading", { level: 3 })
      .map((heading) => heading.textContent);
    expect(headings.slice(0, 3)).toEqual(["Channels", "GitHub", "Triggers"]);
    // GitHub sits beside Triggers, not inside it.
    const triggers = screen.getByRole("heading", { name: "Triggers" }).closest("section");
    expect(triggers).not.toContainElement(screen.getByTestId("github-section"));
    expect(screen.getByTestId("github-section")).toHaveAttribute("data-github-triggers", "0");
  });
});
