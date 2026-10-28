import type { ModelProfile } from "@/lib/api/types";
import { estimateTaskCost, scaleWorkload, type CostWorkload } from "./agent-cost-estimate";

export interface PlanningTask extends CostWorkload {
  name: string;
  description: string;
  assumption: string;
  weight: number;
}

/** Weights describe a user-selected mix, never inferred probabilities. */
export function projectTaskMix(
  tasks: PlanningTask[],
  profile: ModelProfile | undefined,
  definitionTokens: number,
  outputLimit?: number,
) {
  if (
    tasks.some(
      (t) =>
        t.weight > 0 &&
        (!Number.isInteger(t.calls) ||
          t.calls < 1 ||
          t.calls > 100 ||
          ![t.messageTokens, t.outputTokens, t.toolResultTokens].every(
            (v) => Number.isInteger(v) && v >= 0 && v <= 1_000_000,
          ) ||
          t.outputTokens < 1),
    )
  )
    return null;
  if (!tasks.length || tasks.some((t) => !Number.isFinite(t.weight) || t.weight < 0)) return null;
  const total = tasks.reduce((sum, t) => sum + t.weight, 0);
  if (!total) return null;
  const rows = tasks.map((task) => ({
    task,
    share: task.weight / total,
    scenarios: [0.5, 1, 2].map((scale) =>
      estimateTaskCost(profile, definitionTokens, scaleWorkload(task, scale), outputLimit),
    ),
  }));
  const costs = [0, 1, 2].map((index) => {
    if (
      rows.some((r) => r.share > 0 && (!r.scenarios[index] || r.scenarios[index]?.exceedsContext))
    )
      return null;
    return rows.reduce((sum, row) => sum + (row.scenarios[index]?.totalCost ?? 0) * row.share, 0);
  });
  return { rows, low: costs[0], typical: costs[1], high: costs[2] };
}

/** Headline amounts stay readable without displaying tiny positive spend as free. */
export function formatPlanningUsd(value: number, locale: string): string {
  const format = new Intl.NumberFormat(locale, { style: "currency", currency: "USD" });
  return value > 0 && value < 0.01 ? `<${format.format(0.01)}` : format.format(value);
}
