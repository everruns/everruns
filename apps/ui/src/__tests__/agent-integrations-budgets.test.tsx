import { render, screen } from "@testing-library/react";
import { AgentIntegrationsPanel } from "@/components/agents/agent-integrations-panel";
import type { Agent, AppChannel } from "@/lib/api/types";

const mockUseFeatureFlag = jest.fn();
const mockCanAgent = jest.fn();
const mockCanBudget = jest.fn();

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: (flag: string) => mockUseFeatureFlag(flag),
}));

jest.mock("@/hooks/use-agent-endpoints", () => ({
  isTriggerChannel: () => false,
  useAgentEndpoints: () => ({
    endpoints: [
      {
        channel: {
          id: "endpoint_1",
          channel_type: "webhook",
          channel_config: {},
          enabled: true,
          status: "live",
          created_at: "2026-09-19T00:00:00Z",
          updated_at: "2026-09-19T00:00:00Z",
        } satisfies AppChannel,
      },
    ],
    isLoading: false,
  }),
  usePublishAgentEndpoint: () => ({ mutate: jest.fn(), isPending: false }),
  useTriggerAgentEndpoint: () => ({ mutate: jest.fn(), isPending: false }),
}));

jest.mock("@/hooks/use-agent-triggers", () => ({
  useAgentTriggers: () => ({ data: [] }),
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

jest.mock("@/components/apps/channel-row", () => ({
  ChannelRow: ({ usePanel }: { usePanel?: React.ReactNode }) => <div>{usePanel}</div>,
}));

jest.mock("@/components/agents/endpoint-use-panel", () => ({
  EndpointUsePanel: () => <div>Endpoint use</div>,
}));

jest.mock("@/components/agents/agent-triggers-panel", () => ({
  AgentTriggersPanel: () => <div>Agent triggers</div>,
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
    <div
      data-testid={`budget-${subjectType}-${subjectId}`}
      data-can-manage={String(canManage)}
    />
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

    expect(mockUseFeatureFlag).toHaveBeenCalledWith("app_budgets");
    expect(screen.getByTestId("budget-agent-agent_1")).toHaveAttribute(
      "data-can-manage",
      String(canManage),
    );
    expect(screen.getByTestId("budget-agent_endpoint-endpoint_1")).toHaveAttribute(
      "data-can-manage",
      String(canManage),
    );
  });

  it("does not fetch or mount budgets without budget.view", () => {
    mockCanBudget.mockReturnValue(false);

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.queryByTestId(/^budget-/)).not.toBeInTheDocument();
  });

  it("does not mount any budget UI when app_budgets is disabled", () => {
    mockUseFeatureFlag.mockReturnValue(false);
    mockCanBudget.mockReturnValue(true);

    render(<AgentIntegrationsPanel agent={agent} />);

    expect(screen.queryByTestId(/^budget-/)).not.toBeInTheDocument();
  });
});
