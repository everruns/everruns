import {
  buildEvalsOverview,
  liveQualityOf,
  passRateTrend,
  weightedPassRate,
} from "@/components/evals/evals-overview-data";
import { sortFindings } from "@/components/evals/findings";
import type {
  Agent,
  Eval,
  EvalExperiment,
  EvalRun,
  HealthIssue,
  QualitySummary,
} from "@/lib/api/types";

function evalFor(id: string, agentId: string | undefined, passed: number, total: number): Eval {
  return {
    id,
    name: id,
    tags: [],
    status: "active",
    case_count: total,
    target: { type: "session", agent_id: agentId },
    last_run: {
      id: `run_${id}`,
      status: "completed",
      created_at: "2026-11-01T10:00:00Z",
      summary: {
        total,
        passed,
        failed: total - passed,
        errored: 0,
        pass_rate: passed / total,
        avg_score: 0,
        avg_turns: 0,
        avg_latency_ms: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
      },
    },
    created_at: "2026-10-01T00:00:00Z",
    updated_at: "2026-10-01T00:00:00Z",
  };
}

function issue(
  id: string,
  code: string,
  agentId?: string,
  at = "2026-11-01T00:00:00Z",
): HealthIssue {
  return {
    id,
    code,
    status: "open",
    title: id,
    body: "",
    missing_scopes: [],
    first_detected_at: at,
    last_checked_at: at,
    stale: false,
    href: `/health/${id}`,
    agent_id: agentId,
    fixers: [],
  } as HealthIssue;
}

describe("evals overview", () => {
  it("weights the pass rate by trials so a small dataset cannot outvote a large one", () => {
    const small = evalFor("small", "agent_a", 0, 2);
    const large = evalFor("large", "agent_a", 90, 100);
    expect(weightedPassRate([small, large]).rate).toBeCloseTo(90 / 102);
  });

  it("keeps the last finished runs, oldest first", () => {
    const runs = [
      {
        id: "2",
        status: "completed",
        created_at: "2026-11-02",
        summary: { total: 1, pass_rate: 1 },
      },
      {
        id: "1",
        status: "completed",
        created_at: "2026-11-01",
        summary: { total: 1, pass_rate: 0.5 },
      },
      { id: "3", status: "running", created_at: "2026-11-03" },
      {
        id: "4",
        status: "completed",
        created_at: "2026-11-04",
        experiment_id: "evalexp_1",
        summary: { total: 1, pass_rate: 0 },
      },
    ] as unknown as EvalRun[];
    // Experiment variants compare options; they are not the agent's trend.
    expect(passRateTrend(runs)).toEqual([0.5, 1]);
  });

  it("groups datasets by agent and counts only findings", () => {
    const agents = [{ id: "agent_a" }, { id: "agent_b" }, { id: "agent_c" }] as Agent[];
    const overview = buildEvalsOverview({
      evals: [
        evalFor("e1", "agent_a", 8, 10),
        evalFor("e2", "agent_b", 5, 10),
        evalFor("e3", undefined, 1, 1),
      ],
      agents,
      findings: [
        issue("abuse", "session.suspected_abuse", "agent_a"),
        issue("loop", "session.tool_error_loop", "agent_a"),
        issue("token", "connection.token_expired", "agent_a"),
      ],
      experiments: [
        { id: "x", eval_id: "e1", status: "running", created_at: "2026-10-31T00:00:00Z" },
        { id: "old", eval_id: "e1", status: "completed", created_at: "2026-10-01T00:00:00Z" },
      ] as EvalExperiment[],
      runsByEval: new Map(),
      agentName: (agent) => agent.id,
      now: new Date("2026-11-01T12:00:00Z"),
    });
    expect(overview.agentsWithEvals).toBe(2);
    expect(overview.totalAgents).toBe(3);
    expect(overview.openFindings).toBe(2);
    expect(overview.highFindings).toBe(1);
    expect(overview.recentExperiments).toBe(1);
    expect(overview.runningExperiments).toBe(1);
    const a = overview.rows.find((row) => row.agentId === "agent_a")!;
    expect(a.openFindings).toBe(2);
    expect(a.passRate).toBe(0.8);
    expect(a.lastActivityAt).toBe("2026-11-01T10:00:00Z");
  });

  it("joins live quality from observers per agent and in total", () => {
    const quality = {
      since: "2026-10-25T00:00:00Z",
      days: 7,
      scored: 40,
      passed: 30,
      pass_rate: 0.75,
      sessions: 12,
      by_agent: [
        { key: "agent_a", scored: 30, passed: 27, pass_rate: 0.9, sessions: 9, trend: [0.8, 0.9] },
        { key: "agent_b", scored: 0, passed: 0, sessions: 0 },
      ],
      by_observer: [],
      by_day: [
        { key: "2026-10-31", scored: 10, passed: 5, pass_rate: 0.5, sessions: 3 },
        { key: "2026-11-01", scored: 30, passed: 25, pass_rate: 25 / 30, sessions: 9 },
      ],
    } as QualitySummary;
    expect(liveQualityOf(undefined)).toBeNull();
    expect(liveQualityOf(quality)).toEqual({
      rate: 0.75,
      scored: 40,
      sessions: 12,
      trend: [0.5, 25 / 30],
    });
    expect(liveQualityOf(quality, "agent_a")).toEqual({
      rate: 0.9,
      scored: 30,
      sessions: 9,
      trend: [0.8, 0.9],
    });
    // Nothing scored, or no group at all, reads as "not observed".
    expect(liveQualityOf(quality, "agent_b")).toBeNull();
    expect(liveQualityOf(quality, "agent_c")).toBeNull();

    const overview = buildEvalsOverview({
      evals: [evalFor("e1", "agent_a", 8, 10), evalFor("e2", "agent_b", 5, 10)],
      agents: [{ id: "agent_a" }, { id: "agent_b" }] as Agent[],
      findings: [],
      experiments: [],
      runsByEval: new Map(),
      agentName: (agent) => agent.id,
      quality,
    });
    expect(overview.liveQuality?.rate).toBe(0.75);
    expect(overview.rows.find((r) => r.agentId === "agent_a")!.liveQuality?.rate).toBe(0.9);
    expect(overview.rows.find((r) => r.agentId === "agent_b")!.liveQuality).toBeNull();
  });

  it("lists the loudest findings first, then the newest", () => {
    const sorted = sortFindings([
      issue("old-medium", "session.agent_stuck", undefined, "2026-10-30T00:00:00Z"),
      issue("new-medium", "session.spend_spike", undefined, "2026-11-01T00:00:00Z"),
      issue("high", "schedule.runaway", undefined, "2026-10-01T00:00:00Z"),
      issue("other", "connection.token_expired"),
    ]);
    expect(sorted.map((i) => i.id)).toEqual(["high", "new-medium", "old-medium"]);
  });
});
