import { fireEvent, render, screen } from "@testing-library/react";
import {
  CaseComparisonTable,
  VariantTable,
  VerdictNotice,
} from "@/components/evals/experiment-comparison";
import type { EvalExperiment, RunSummary } from "@/lib/api/types";

function summary(passRate: number, every: number): RunSummary {
  return {
    total: 6,
    passed: Math.round(passRate * 6),
    failed: 0,
    errored: 0,
    pass_rate: passRate,
    avg_score: passRate,
    avg_turns: 3.2,
    avg_latency_ms: 14000,
    total_input_tokens: 0,
    total_output_tokens: 0,
    cases: 2,
    cases_passed_every_trial: every,
  };
}

const experiment: EvalExperiment = {
  id: "evalexp_1",
  eval_id: "eval_1",
  name: "base vs sol",
  dataset_version: 7,
  trials: 3,
  baseline: "base",
  status: "completed",
  created_at: "2026-11-01T00:00:00Z",
  variants: [
    {
      label: "base",
      run_id: "r1",
      status: "completed",
      summary: summary(0.5, 1),
      cost_per_case_usd: 0.031,
    },
    {
      label: "sol",
      run_id: "r2",
      status: "completed",
      model_override: "model_sol",
      summary: summary(1, 2),
      cost_per_case_usd: 0.035,
    },
  ],
  comparison: {
    baseline: "base",
    verdicts: [{ label: "sol", paired_cases: 2, fixed: 1, broke: 0, verdict: "not_enough_data" }],
    cases: [
      {
        case_id: "c1",
        case_name: "Ticket in Ukrainian",
        changed: true,
        cells: [
          { label: "base", passed: 0, trials: 3, cost_usd: 0.02 },
          { label: "sol", passed: 3, trials: 3, cost_usd: 0.02 },
        ],
      },
      {
        case_id: "c2",
        case_name: "Duplicate ticket",
        changed: false,
        cells: [
          { label: "base", passed: 3, trials: 3, cost_usd: 0.01 },
          { label: "sol", passed: 3, trials: 3, cost_usd: 0.01 },
        ],
      },
    ],
  },
};

it("shows each variant against the baseline with model names and deltas", () => {
  render(
    <VariantTable experiment={experiment} modelNames={new Map([["model_sol", "GPT-6.1 Sol"]])} />,
  );
  expect(screen.getByText("GPT-6.1 Sol")).toBeInTheDocument();
  // Pass rate and passes-every-trial both gained 50 points.
  expect(screen.getAllByText("+50")).toHaveLength(2);
  expect(screen.getByText("+13%")).toBeInTheDocument(); // cost per case, relative
});

it("phrases the verdict", () => {
  render(<VerdictNotice verdict={experiment.comparison!.verdicts[0]} />);
  expect(screen.getByTestId("experiment-verdict")).toHaveTextContent(
    "Not enough data yet. sol fixed 1 case and broke 0 cases.",
  );
});

it("shows changed cases first and all cases on request", () => {
  render(<CaseComparisonTable experiment={experiment} />);
  expect(screen.getByText("Ticket in Ukrainian")).toBeInTheDocument();
  expect(screen.queryByText("Duplicate ticket")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "All" }));
  expect(screen.getByText("Duplicate ticket")).toBeInTheDocument();
});
