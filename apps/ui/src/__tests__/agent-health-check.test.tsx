import { render, screen } from "@testing-library/react";
import { AgentHealthCheck } from "@/components/agents/agent-health-check";
import type { HealthCheckRun, LatestHealthCheckRun } from "@/lib/api/types";

const mockTrigger = jest.fn(() => ({ mutate: jest.fn(), isPending: false, error: null }));
let mockRun: HealthCheckRun | undefined;
let mockLatest: LatestHealthCheckRun | undefined;

jest.mock("@/hooks/use-agents", () => ({
  useTriggerHealthCheck: () => mockTrigger(),
  useHealthCheckRun: () => ({ data: mockRun }),
  useLatestHealthCheckRun: () => ({ data: mockLatest }),
}));

const completedRun: HealthCheckRun = {
  id: "healthcheck_1",
  config_hash: "abc",
  status: "completed",
  created_at: "2026-06-13T00:00:00Z",
  summary: {
    total: 2,
    passed: 1,
    failed: 1,
    errored: 0,
    pass_rate: 0.5,
    avg_score: 0.7,
    avg_turns: 1.5,
    total_input_tokens: 0,
    total_output_tokens: 0,
  },
  results: [
    {
      name: "greeting",
      user_message: "Hi",
      rubric: "Polite",
      session_id: "session_1",
      passed: true,
      score: 0.9,
      judge_reason: "Responded politely.",
      deterministic_reason: "Completed.",
      turns: 1,
      latency_ms: 100,
    },
    {
      name: "edge",
      user_message: "???",
      rubric: "Clarifies",
      passed: false,
      score: 0.4,
      judge_reason: "Did not clarify.",
      deterministic_reason: "Completed.",
      turns: 2,
      latency_ms: 200,
    },
  ],
};

describe("AgentHealthCheck", () => {
  beforeEach(() => {
    mockRun = undefined;
    mockLatest = undefined;
  });

  it("shows an idle empty state with a run button", () => {
    render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByRole("button", { name: /run health check/i })).toBeInTheDocument();
    expect(screen.getByText(/no health check has been run/i)).toBeInTheDocument();
  });

  it("renders the score card and per-case results when completed", () => {
    mockRun = completedRun;
    render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByText("50%")).toBeInTheDocument();
    expect(screen.getByText("greeting")).toBeInTheDocument();
    expect(screen.getByText("Responded politely.")).toBeInTheDocument();
    expect(screen.getByText("Did not clarify.")).toBeInTheDocument();
  });

  it("shows the failure reason when the run failed", () => {
    mockRun = {
      id: "healthcheck_2",
      config_hash: "abc",
      status: "failed",
      created_at: "2026-06-13T00:00:00Z",
      error_message: "utility LLM down",
    };
    render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByText(/utility LLM down/i)).toBeInTheDocument();
  });

  it("shows the latest persisted run on mount without triggering one", () => {
    // No freshly triggered run; only the latest run loaded on mount (EVE-588).
    mockLatest = { run: completedRun, config_changed: false };
    render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByText("50%")).toBeInTheDocument();
    expect(screen.getByText("greeting")).toBeInTheDocument();
    expect(screen.queryByText(/no health check has been run/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/configuration changed since this run/i)).not.toBeInTheDocument();
  });

  it("shows a stale-config hint when the latest run predates the current config", () => {
    mockLatest = { run: completedRun, config_changed: true };
    render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByText(/configuration changed since this run/i)).toBeInTheDocument();
    // The prior run is still rendered alongside the hint.
    expect(screen.getByText("50%")).toBeInTheDocument();
  });

  it("keeps the run button disabled while the latest run is still in progress", () => {
    // A run loaded on mount that is still running must not allow a duplicate
    // trigger, even though nothing was triggered in this session.
    mockLatest = {
      run: {
        id: "healthcheck_3",
        config_hash: "abc",
        status: "running",
        created_at: "2026-06-13T00:00:00Z",
      },
      config_changed: false,
    };
    render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByRole("button", { name: /running/i })).toBeDisabled();
  });
});
