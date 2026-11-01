// Numbers for the Evals overview and an agent's Evals tab, computed from the
// lists the pages already load. Pure so tests can pin the arithmetic.
//
// Pass rates are weighted by trials, not averaged per dataset, so a 3-case
// dataset cannot outvote a 300-case one.

import type { Agent, Eval, EvalExperiment, EvalRun, HealthIssue } from "@/lib/api/types";
import { findingSeverity, isFinding } from "./findings";

export const EXPERIMENT_WINDOW_DAYS = 7;
const TREND_POINTS = 8;

/** The agent a dataset tests, when its target is a session on an agent. */
export function evalAgentId(ev: Eval): string | undefined {
  return ev.target?.type === "session" ? (ev.target.agent_id ?? undefined) : undefined;
}

/** Trial-weighted pass rate over the latest finished run of each dataset. */
export function weightedPassRate(evals: Eval[]): {
  rate: number | null;
  cases: number;
} {
  let passed = 0;
  let total = 0;
  let cases = 0;
  for (const ev of evals) {
    const summary = ev.last_run?.summary;
    if (!summary || summary.total === 0) continue;
    passed += summary.passed;
    total += summary.total;
    cases += summary.cases ?? summary.total;
  }
  return { rate: total > 0 ? passed / total : null, cases };
}

/** Pass rates of finished runs, oldest first, at most the last few. */
/**
 * A finished run of the dataset as configured. Experiment variants run with a
 * model or target override, so they compare options rather than track the
 * agent, and stay out of trends.
 */
export function isTrendRun(run: EvalRun): boolean {
  return run.status === "completed" && !run.experiment_id && !!run.summary && run.summary.total > 0;
}

export function passRateTrend(runs: EvalRun[]): number[] {
  return runs
    .filter(isTrendRun)
    .sort((a, b) => a.created_at.localeCompare(b.created_at))
    .slice(-TREND_POINTS)
    .map((run) => run.summary!.pass_rate);
}

export interface AgentEvalRow {
  agentId: string;
  name: string;
  datasets: Eval[];
  passRate: number | null;
  trend: number[];
  openFindings: number;
  lastActivityAt: string | null;
}

export interface EvalsOverview {
  agentsWithEvals: number;
  totalAgents: number;
  passRate: number | null;
  passRateCases: number;
  recentExperiments: number;
  runningExperiments: number;
  openFindings: number;
  highFindings: number;
  rows: AgentEvalRow[];
}

export function buildEvalsOverview({
  evals,
  agents,
  findings,
  experiments,
  runsByEval,
  agentName,
  now = new Date(),
}: {
  evals: Eval[];
  agents: Agent[];
  findings: HealthIssue[];
  experiments: EvalExperiment[];
  runsByEval: Map<string, EvalRun[]>;
  agentName: (agent: Agent) => string;
  now?: Date;
}): EvalsOverview {
  const active = evals.filter((ev) => ev.status === "active");
  const byAgent = new Map<string, Eval[]>();
  for (const ev of active) {
    const agentId = evalAgentId(ev);
    if (!agentId) continue;
    byAgent.set(agentId, [...(byAgent.get(agentId) ?? []), ev]);
  }
  const open = findings.filter(isFinding);
  const since = now.getTime() - EXPERIMENT_WINDOW_DAYS * 24 * 60 * 60 * 1000;
  const recent = experiments.filter((e) => new Date(e.created_at).getTime() >= since);
  const evalIdsByAgent = new Map(
    [...byAgent.entries()].map(([agentId, list]) => [agentId, new Set(list.map((e) => e.id))]),
  );

  const rows: AgentEvalRow[] = [...byAgent.entries()].map(([agentId, datasets]) => {
    const agent = agents.find((a) => a.id === agentId);
    const runs = datasets.flatMap((ev) => runsByEval.get(ev.id) ?? []);
    const ids = evalIdsByAgent.get(agentId)!;
    const activity = [
      ...datasets.map((ev) => ev.last_run?.created_at),
      ...experiments.filter((e) => ids.has(e.eval_id)).map((e) => e.created_at),
    ].filter((at): at is string => !!at);
    return {
      agentId,
      name: agent ? agentName(agent) : agentId,
      datasets,
      passRate: weightedPassRate(datasets).rate,
      trend: passRateTrend(runs),
      openFindings: open.filter((f) => f.agent_id === agentId).length,
      lastActivityAt: activity.sort().at(-1) ?? null,
    };
  });
  rows.sort((a, b) => (b.lastActivityAt ?? "").localeCompare(a.lastActivityAt ?? ""));

  const overall = weightedPassRate(active);
  return {
    agentsWithEvals: byAgent.size,
    totalAgents: agents.length,
    passRate: overall.rate,
    passRateCases: overall.cases,
    recentExperiments: recent.length,
    runningExperiments: experiments.filter((e) => e.status === "running").length,
    openFindings: open.length,
    highFindings: open.filter((f) => findingSeverity(f) === "high").length,
    rows,
  };
}
