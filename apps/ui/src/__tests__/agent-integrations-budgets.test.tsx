import { render, screen } from "@testing-library/react";
import { AgentIntegrationsPanel } from "@/components/agents/agent-integrations-panel";
import type { Agent, AgentChannel } from "@/lib/api/types";

const mockUseFeatureFlag = jest.fn();
const mockCanAgent = jest.fn();
const mockCanBudget = jest.fn();

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

jest.mock("@/components/agents/integrations/channel-details-panel", () => ({
  ChannelDetailsPanel: ({ channel }: { channel: AgentChannel }) => (
    <div data-testid={`channel-details-${channel.id}`} />
  ),
}));

jest.mock("@/components/agents/agent-github-card", () => ({
  AgentGitHubCard: () => <div>Agent GitHub</div>,
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
  ])("gives $role the budget controls allowed by policy", ({ canManage }) => {
    mockCanBudget.mockImplementation((policy: string) =>
      policy === "budget.view" ? true : canManage,
    );

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(mockUseFeatureFlag).toHaveBeenCalledWith("channel_budgets");
    expect(screen.getByTestId("channel-details-endpoint_1")).toBeInTheDocument();
    expect(screen.getByTestId("budget-agent-agent_1")).toHaveAttribute(
      "data-can-manage",
      String(canManage),
    );
    expect(screen.getByTestId("budget-agent_channel-endpoint_1")).toHaveAttribute(
      "data-can-manage",
      String(canManage),
    );
  });

  it("does not fetch or mount budgets without budget.view", () => {
    mockCanBudget.mockReturnValue(false);

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.queryByTestId(/^budget-/)).not.toBeInTheDocument();
  });

  it("does not mount any budget UI when channel_budgets is disabled", () => {
    mockUseFeatureFlag.mockReturnValue(false);
    mockCanBudget.mockReturnValue(true);

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.queryByTestId(/^budget-/)).not.toBeInTheDocument();
  });

  it("enables or disables every channel and keeps each channel's publish state", () => {
    mockCanBudget.mockReturnValue(false);

    const { rerender } = render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.getByRole("switch", { name: "Enabled" })).toBeChecked();
    expect(
      screen.getByText(
        "Off pauses every channel on this agent. Publish on each channel stays as it is.",
      ),
    ).toBeInTheDocument();

    rerender(<AgentIntegrationsPanel agent={{ ...agent, exposures_suspended: true }} />);

    expect(screen.getByRole("switch", { name: "Enabled" })).not.toBeChecked();
    expect(screen.getByText("Disabled")).toBeInTheDocument();
  });

  it("opens the channel named in the address", () => {
    mockCanBudget.mockReturnValue(false);

    render(<AgentIntegrationsPanel agent={agent} initialChannelId="endpoint_1" />);

    expect(screen.getByTestId("row-endpoint_1")).toHaveAttribute("data-expanded", "true");
  });

  it("groups GitHub setup and trigger creation under one Triggers heading", () => {
    mockUseFeatureFlag.mockReturnValue(false);
    mockCanBudget.mockReturnValue(false);

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.getAllByText("Triggers")).toHaveLength(1);
    expect(screen.getByRole("link", { name: "Add trigger" })).toHaveAttribute(
      "href",
      "/agents/agent_1/triggers/new",
    );
    expect(screen.getByText("Agent GitHub")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "No triggers yet" })).toBeInTheDocument();
  });
});
