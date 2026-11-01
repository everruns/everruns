import { render, screen, within } from "@testing-library/react";
import { OrgHome } from "@/components/home/org-home";

let mockAgents: unknown[] = [];
let mockActivity: unknown = undefined;
let mockIssues: unknown[] = [];
let mockPausedTotal = 0;
const mockChatRows = jest.fn();

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ href, children, ...props }: { href: string; children: React.ReactNode }) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));
jest.mock("@tanstack/react-query", () => ({
  useQuery: () => ({ data: undefined, isError: false, isLoading: false }),
}));
jest.mock("@/components/charts/time-series-chart", () => ({
  TimeSeriesChart: ({ ariaLabel }: { ariaLabel: string }) => (
    <div role="img" aria-label={ariaLabel} />
  ),
  ChartLegend: () => null,
}));
jest.mock("@/components/agents/new-agent-link", () => ({
  NewAgentLink: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
jest.mock("@/components/chat-mode/page-view-state", () => ({
  useChatPageRows: (...args: unknown[]) => mockChatRows(...args),
}));
jest.mock("@/hooks", () => ({
  useAgents: () => ({ data: mockAgents, isLoading: false }),
  useAgentExamples: () => ({ data: [] }),
  usePageTitle: () => undefined,
}));
jest.mock("@/hooks/use-agent-activity", () => ({
  useAgentActivity: () => ({ data: mockActivity, isLoading: false }),
}));
jest.mock("@/hooks/use-health-issues", () => ({
  useHealthIssues: () => ({ data: { data: mockIssues }, isLoading: false }),
}));
jest.mock("@/hooks/use-org-exposures", () => ({
  useOrgExposures: () => ({ exposures: [], isLoading: false }),
}));
jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ isLoading: false, error: null, can: () => false }),
}));
jest.mock("@/hooks/use-providers", () => ({
  useModels: () => ({ data: [{ id: "m", enabled: true, display_name: "Luna" }] }),
}));
jest.mock("@/hooks/use-sandbox-fleet", () => ({
  useSandboxFleetStats: () => ({ data: undefined }),
}));
jest.mock("@/hooks/use-sessions", () => ({
  useFilteredSessions: (filters: { status: string[] }) => ({
    data:
      filters.status[0] === "paused"
        ? {
            data: mockPausedTotal
              ? [{ id: "s1", title: "Q3 scan", updated_at: "2026-11-01T10:00:00Z" }]
              : [],
            total: mockPausedTotal,
          }
        : { data: [], total: 0 },
  }),
}));
jest.mock("@/hooks/use-system-doctor", () => ({ useSystemDoctor: () => ({ data: undefined }) }));
jest.mock("@/providers/feature-flags-provider", () => ({ useFeatureFlag: () => false }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1", name: "Alice's org" } }),
}));

const agent = (id: string, name: string) => ({
  id,
  name,
  display_name: name,
  status: "active",
  is_built_in: false,
  channels: [],
  tags: [],
  created_at: "2026-10-01T00:00:00Z",
});

beforeEach(() => {
  mockAgents = [];
  mockActivity = undefined;
  mockIssues = [];
  mockPausedTotal = 0;
  mockChatRows.mockClear();
});

it("guides a new organization instead of showing empty numbers", () => {
  render(<OrgHome />);
  expect(screen.getByText("Get started")).toBeInTheDocument();
  expect(screen.getByText("Nothing needs attention.")).toBeInTheDocument();
  expect(screen.queryByTestId("home-kpis")).not.toBeInTheDocument();
});

it("lists what needs attention, gives Chat the same rows, and shows the agents", () => {
  mockAgents = [agent("agent_a", "Release notes bot")];
  mockPausedTotal = 1;
  mockIssues = [
    {
      id: "limit",
      code: "org.active_turn_limit",
      status: "open",
      title: "At the active-turn limit",
      body: "New turns queue.",
      href: "/settings/health?issue=limit",
      agent_id: null,
      first_detected_at: "2026-11-01T11:00:00Z",
      missing_scopes: [],
      snoozed_until: null,
    },
  ];
  const hourly = Array.from({ length: 24 }, (_, i) =>
    i >= 22 ? { runs: 4, failed: 3 } : { runs: 2, failed: 0 },
  );
  mockActivity = {
    agents: [
      {
        agent_id: "agent_a",
        hourly,
        runs: 52,
        failed: 6,
        running_sessions: 1,
        last_turn_at: "2026-11-01T11:59:00Z",
        triggers: [],
      },
    ],
    channels: [],
    totals: {
      running_sessions: 40,
      max_active_turns: 40,
      runs: 52,
      failed: 6,
      channel_sessions: 3,
      people_reached: 2,
    },
    window_start: "2026-10-31T12:00:00Z",
    generated_at: "2026-11-01T12:00:00Z",
  };
  render(<OrgHome />);

  const attention = screen.getByTestId("home-attention");
  const rows = within(attention).getAllByRole("listitem");
  expect(rows.map((row) => row.dataset.severity)).toEqual(["error", "error", "warning"]);
  expect(within(attention).getByText("1 session paused at its budget limit")).toBeInTheDocument();
  expect(
    within(attention).getByText("6 of 8 turns failed in the last 2 hours"),
  ).toBeInTheDocument();
  expect(within(attention).getByText("At the active-turn limit")).toBeInTheDocument();

  const [label, chatRows] = mockChatRows.mock.calls.at(-1);
  expect(label).toBe("Needs attention");
  expect(chatRows).toHaveLength(3);

  expect(
    within(screen.getByTestId("home-kpis")).getByText("40 of 40 turn limit"),
  ).toBeInTheDocument();
  expect(
    within(screen.getByTestId("home-agents")).getByText("Release notes bot"),
  ).toBeInTheDocument();
  // Without Monitoring, the hourly agents buckets still draw the day.
  expect(screen.getByRole("img", { name: "Turns over time" })).toBeInTheDocument();
});
