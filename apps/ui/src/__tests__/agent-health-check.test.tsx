import { fireEvent, render, screen } from "@testing-library/react";
import { AgentHealthCheck } from "@/components/agents/agent-health-check";
import type { HealthCheckRun, LatestHealthCheckRun } from "@/lib/api/types";

const mockTrigger = jest.fn(() => ({ mutate: jest.fn(), isPending: false, error: null }));
const mockUseHealthCheckRun = jest.fn((_agentId: string, _runId: string | null) => ({
  data: mockRun,
}));
let mockRun: HealthCheckRun | undefined;
let mockLatest: LatestHealthCheckRun | undefined;

jest.mock("@/hooks/use-agents", () => ({
  useTriggerHealthCheck: () => mockTrigger(),
  useHealthCheckRun: (agentId: string, runId: string | null) =>
    mockUseHealthCheckRun(agentId, runId),
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
    total_input_tokens: 1500,
    total_output_tokens: 300,
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
    jest.clearAllMocks();
    mockRun = undefined;
    mockLatest = undefined;
  });

  it("shows an idle empty state with a run button", () => {
    render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByRole("button", { name: /run health check/i })).toBeInTheDocument();
    expect(screen.getByText(/no health check has been run/i)).toBeInTheDocument();
  });

  it("starts a run from the compact rail action", () => {
    const mutate = jest.fn();
    mockTrigger.mockReturnValue({ mutate, isPending: false, error: null });
    render(<AgentHealthCheck agentId="agent_1" />);

    fireEvent.click(screen.getByRole("button", { name: /run health check/i }));

    expect(mutate).toHaveBeenCalledWith(
      "agent_1",
      expect.objectContaining({ onSuccess: expect.any(Function) }),
    );
  });

  it("renders the score card and per-case results when completed", () => {
    mockRun = completedRun;
    render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByText("50%")).toBeInTheDocument();
    expect(screen.getByText("Case results (2)")).toBeInTheDocument();
    expect(screen.getByText("greeting")).toBeInTheDocument();
    expect(screen.getByText("Responded politely.")).toBeInTheDocument();
    expect(screen.getByText("Did not clarify.")).toBeInTheDocument();
    // Token usage from the summary is surfaced.
    expect(screen.getByText(/1,500 input/)).toBeInTheDocument();
    expect(screen.getByText(/300 output tokens/)).toBeInTheDocument();
  });

  it("uses a rail-safe two-column summary and wrapped expandable details", () => {
    mockRun = completedRun;
    render(<AgentHealthCheck agentId="agent_1" />);

    expect(screen.getByText("Pass rate").parentElement?.parentElement).toHaveClass("grid-cols-2");
    const details = screen.getByText("Case results (2)").closest("details");
    expect(details).toHaveClass("min-w-0");
    expect(screen.getByText("Responded politely.")).toHaveClass("break-words");
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
    expect(mockUseHealthCheckRun).toHaveBeenCalledWith("agent_1", "healthcheck_3");
  });

  it("renders a polled terminal failure live and clears the running state", () => {
    mockLatest = {
      run: {
        id: "healthcheck_4",
        config_hash: "abc",
        status: "running",
        created_at: "2026-06-13T00:00:00Z",
      },
      config_changed: false,
    };
    const view = render(<AgentHealthCheck agentId="agent_1" />);
    expect(screen.getByRole("button", { name: /running/i })).toBeDisabled();

    mockRun = {
      id: "healthcheck_4",
      config_hash: "abc",
      status: "failed",
      created_at: "2026-06-13T00:00:00Z",
      error_message:
        "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue.",
    };
    view.rerender(<AgentHealthCheck agentId="agent_1" />);

    expect(screen.getByText(/out of credits or quota/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /run health check/i })).toBeEnabled();
    expect(screen.queryByText(/generating and running cases/i)).not.toBeInTheDocument();
  });
});
