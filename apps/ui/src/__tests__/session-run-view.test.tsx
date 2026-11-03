import { render, screen } from "@testing-library/react";
import { LocaleProvider } from "@/providers/locale-provider";
import type { SessionRun } from "@/lib/api/schema-types";
import {
  RunCostSplit,
  RunHandoffs,
  RunSummaryCards,
  RunTimeline,
} from "@/components/session/run-view";
import { RunsWithAgentsCard } from "@/components/costs/costs-overview";
import { buildSessionNavigation } from "@/components/session/session-header";

const run: SessionRun = {
  root_session_id: "session_lead",
  cost_usd: 5.06,
  first_session_cost_usd: 0.25,
  waiting_requests: 1,
  truncated: false,
  sessions: [
    {
      session_id: "session_lead",
      agent: { id: "agent_lead", name: "Foreman" },
      title: "Fix flaky login refresh",
      status: "active",
      created_at: "2026-11-03T10:00:00Z",
      cost_usd: 0.25,
      turns: [
        {
          turn_no: 1,
          turn_id: "turn_1",
          started_at: "2026-11-03T10:00:00Z",
          ended_at: "2026-11-03T10:00:30Z",
          status: "completed",
          cost_usd: 0.06,
        },
      ],
    },
    {
      session_id: "session_triage",
      parent_session_id: "session_lead",
      agent: { id: "agent_triage", name: "Triage" },
      title: "Reproduce flaky login test",
      status: "idle",
      last_turn_status: "completed",
      created_at: "2026-11-03T10:00:20Z",
      cost_usd: 0.21,
      turns: [
        {
          turn_no: 1,
          turn_id: "turn_2",
          started_at: "2026-11-03T10:00:20Z",
          ended_at: "2026-11-03T10:02:00Z",
          status: "completed",
          cost_usd: 0.21,
        },
      ],
    },
    {
      session_id: "session_impl",
      parent_session_id: "session_lead",
      agent: { id: "agent_impl", name: "Implement" },
      title: "Implement the spec",
      status: "waiting_for_tool_results",
      created_at: "2026-11-03T10:02:10Z",
      cost_usd: 4.6,
      turns: [
        {
          turn_no: 1,
          turn_id: "turn_3",
          started_at: "2026-11-03T10:02:10Z",
          status: "running",
          cost_usd: 4.6,
        },
      ],
    },
  ],
};

function wrap(node: React.ReactNode) {
  return render(<LocaleProvider>{node}</LocaleProvider>);
}

describe("whole run view", () => {
  it("summarizes the run's cost and what waits on the person", () => {
    wrap(<RunSummaryCards run={run} />);
    expect(screen.getByText("$5.06")).toBeInTheDocument();
    expect(screen.getByText(/Foreman \$0.25 · other agents \$4.81/)).toBeInTheDocument();
    expect(screen.getByText("1 request")).toBeInTheDocument();
  });

  it("lists every hand-off with status and a link to its session", () => {
    wrap(<RunHandoffs run={run} sessionId="session_lead" />);
    expect(screen.getByRole("link", { name: "Triage" })).toHaveAttribute(
      "href",
      "/sessions/session_triage/trace",
    );
    expect(screen.getByText("Done")).toBeInTheDocument();
    expect(screen.getByText("Waiting on the person")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Foreman" })).not.toBeInTheDocument();
  });

  it("draws one lane per agent with a bar per turn", () => {
    wrap(<RunTimeline run={run} sessionId="session_lead" />);
    for (const name of ["Foreman", "Triage", "Implement"]) {
      expect(screen.getByText(name)).toBeInTheDocument();
    }
    expect(screen.getAllByRole("link")).toHaveLength(3);
  });

  it("splits the run's cost by agent, largest first", () => {
    wrap(<RunCostSplit run={run} />);
    const rows = screen.getAllByRole("row").slice(1);
    expect(rows[0]).toHaveTextContent("Implement");
    expect(rows[2]).toHaveTextContent("Foreman");
  });

  it("shows the Agents tab only for sessions in a run", () => {
    const keys = (features: string[], inRun = false) =>
      buildSessionNavigation({ basePath: "/s", features: new Set(features), inRun }).map(
        (item) => item.key,
      );
    expect(keys([])).not.toContain("agents");
    expect(keys(["agent_handoffs"])).toContain("agents");
    expect(keys([], true)).toContain("agents");
  });
});

describe("runs that use other agents", () => {
  it("shows each lead with who spent its runs and notes shared agents", () => {
    wrap(
      <RunsWithAgentsCard
        lines={[
          {
            agent_id: "agent_foreman",
            agent_name: "Foreman",
            runs: 412,
            cost_usd: 212.4,
            by_agent: [
              { agent_id: "agent_impl", agent_name: "Implement", relation: "created_here", cost_usd: 121.3 },
              { agent_id: "agent_review", agent_name: "Review", relation: "linked", cost_usd: 51.75 },
              { agent_id: "agent_foreman", agent_name: "Foreman", relation: "lead", cost_usd: 39.35 },
            ],
          },
          {
            agent_id: "agent_support",
            agent_name: "Support",
            runs: 3,
            cost_usd: 22.9,
            by_agent: [
              { agent_id: "agent_review", agent_name: "Review", relation: "linked", cost_usd: 22.9 },
            ],
          },
        ]}
      />,
    );
    expect(screen.getByText("412 runs")).toBeInTheDocument();
    expect(screen.getByText("Foreman itself")).toBeInTheDocument();
    expect(screen.getByText("Created here")).toBeInTheDocument();
    expect(screen.getAllByText("Linked")).toHaveLength(2);
    expect(screen.getByText(/Review worked for 2 agents/)).toBeInTheDocument();
  });
});
