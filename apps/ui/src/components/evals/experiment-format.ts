// Wording and numbers for experiment views (Evals section, experiment page,
// agent Evals tab), kept in one place so every view says the same thing.
//
// The server decides the verdict (paired bootstrap over cases, see
// knowledge/evaluation/experiments.md); these helpers only phrase it.

import type {
  EvalExperiment,
  ExperimentVariant,
  RunSummary,
  VariantVerdict,
  Verdict,
} from "@/lib/api/types";

type Translate = (message: string, values?: Record<string, unknown>) => string;

/** Signed whole number of points: `+5`, `−3`, `0`. */
export function formatPoints(points: number): string {
  const rounded = Math.round(points);
  if (rounded > 0) return `+${rounded}`;
  if (rounded < 0) return `−${Math.abs(rounded)}`;
  return "0";
}

/** A 0..1 rate as a whole percentage. */
export function formatPercent(rate: number | null | undefined): string {
  if (rate === null || rate === undefined || !Number.isFinite(rate)) return "—";
  return `${Math.round(rate * 100)}%`;
}

/** Share of cases a run passed in every trial (pass^k). */
export function passedEveryTrial(summary?: RunSummary | null): number | null {
  if (!summary || !summary.cases) return null;
  return (summary.cases_passed_every_trial ?? 0) / summary.cases;
}

export function costPerCase(variant: ExperimentVariant): number | null {
  return variant.cost_per_case_usd ?? null;
}

/** Relative change of `value` against `base`, as a signed whole percentage. */
export function relativeChange(value: number | null, base: number | null): number | null {
  if (value === null || base === null || base <= 0) return null;
  return Math.round(((value - base) / base) * 100);
}

/** Whether a change in this direction is good: cost going up is bad. */
export function deltaTone(delta: number, higherIsBetter: boolean): "good" | "bad" | "neutral" {
  if (delta === 0) return "neutral";
  return delta > 0 === higherIsBetter ? "good" : "bad";
}

export function verdictHeadline(verdict: Verdict, ui: Translate): string {
  switch (verdict) {
    case "improvement":
      return ui("Likely a real improvement.");
    case "regression":
      return ui("Likely a real regression.");
    case "within_noise":
      return ui("No clear difference.");
    case "not_enough_data":
      return ui("Not enough data yet.");
  }
}

/** One sentence after the headline: what changed and why we believe it. */
export function verdictDetail(v: VariantVerdict, ui: Translate): string {
  const parts: string[] = [];
  if (v.fixed > 0 || v.broke > 0) {
    parts.push(
      ui("{label} fixed {fixed} and broke {broke}.", {
        label: v.label,
        fixed: casesWord(v.fixed, ui),
        broke: casesWord(v.broke, ui),
      }),
    );
  }
  const low = v.interval_low_points;
  const high = v.interval_high_points;
  const interval =
    low !== undefined && low !== null && high !== undefined && high !== null
      ? ui("95% interval {low} to {high} points", {
          low: formatPoints(low),
          high: formatPoints(high),
        })
      : null;
  switch (v.verdict) {
    case "improvement":
      parts.push(
        ui("The pass rate gain is outside run-to-run noise ({interval}).", {
          interval,
        }),
      );
      break;
    case "regression":
      parts.push(
        ui("The pass rate loss is outside run-to-run noise ({interval}).", {
          interval,
        }),
      );
      break;
    case "within_noise":
      parts.push(
        ui("The difference could be run-to-run noise ({interval}).", {
          interval,
        }),
      );
      break;
    case "not_enough_data":
      parts.push(
        ui(
          "{count} paired cases is too few to tell noise from a change; three or more are needed.",
          {
            count: v.paired_cases,
          },
        ),
      );
      break;
  }
  return parts.join(" ");
}

function casesWord(count: number, ui: Translate): string {
  return count === 1 ? ui("1 case") : ui("{count} cases", { count });
}

/** The verdict worth showing for an experiment in a list: its last variant's. */
export function headlineVerdict(experiment: EvalExperiment): VariantVerdict | undefined {
  const verdicts = experiment.comparison?.verdicts ?? [];
  return verdicts[verdicts.length - 1];
}

/** Total spend across the experiment's finished variant runs. */
export function experimentCost(experiment: EvalExperiment): number | null {
  const costs = experiment.variants
    .map((variant) => variant.summary?.total_cost_usd)
    .filter((cost): cost is number => typeof cost === "number");
  if (costs.length === 0) return null;
  return costs.reduce((sum, cost) => sum + cost, 0);
}

/** Cases in the dataset the experiment ran, from any variant's summary. */
export function experimentCaseCount(experiment: EvalExperiment): number | null {
  for (const variant of experiment.variants) {
    if (variant.summary?.cases) return variant.summary.cases;
  }
  return experiment.comparison?.cases.length ?? null;
}
