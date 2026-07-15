import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { RunComparison } from "@/components/evals/run-comparison";
import type { EvalCaseResult, EvalRun } from "@/lib/api/types";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: ReactNode; href: string }) => (
    // eslint-disable-next-line @next/next/no-html-link-for-pages
    <a href={href}>{children}</a>
  ),
}));

function result(caseName: string, status: EvalCaseResult["status"], value: number): EvalCaseResult {
  return {
    id: `evalresult_${caseName}_${status}`,
    eval_case_id: `evalcase_${caseName}`,
    case_name: caseName,
    status,
    scores: [{ scorer: "contains", value, pass: status === "passed" }],
    created_at: "2026-04-19T10:00:00Z",
    updated_at: "2026-04-19T10:00:00Z",
  };
}

function run(id: string, createdAt: string, results: EvalCaseResult[], passRate: number): EvalRun {
  return {
    id,
    status: "completed",
    triggered_by: "user",
    results,
    summary: {
      total: results.length,
      passed: results.filter((r) => r.status === "passed").length,
      failed: results.filter((r) => r.status === "failed").length,
      errored: 0,
      pass_rate: passRate,
      avg_score: 0,
      avg_turns: 0,
      avg_latency_ms: 0,
      total_input_tokens: 0,
      total_output_tokens: 0,
    },
    created_at: createdAt,
    updated_at: createdAt,
  };
}

describe("RunComparison", () => {
  it("flags a case that regressed between consecutive runs", () => {
    const older = run("evalrun_old", "2026-04-19T10:00:00Z", [result("a", "passed", 1)], 1);
    const newer = run("evalrun_new", "2026-04-20T10:00:00Z", [result("a", "failed", 0)], 0);

    // Pass the runs out of chronological order to exercise the internal sort.
    render(<RunComparison evalId="eval_1" runs={[newer, older]} />);

    // The newer run's cell for case "a" is a regression (passed → failed).
    expect(screen.getByTitle("Regressed from previous run")).toBeInTheDocument();
    // Pass-rate row renders both runs.
    expect(screen.getByText("100%")).toBeInTheDocument();
    expect(screen.getByText("0%")).toBeInTheDocument();
    // Case row label is present.
    expect(screen.getByText("a")).toBeInTheDocument();
  });

  it("flags an improvement (failed → passed)", () => {
    const older = run("evalrun_old", "2026-04-19T10:00:00Z", [result("a", "failed", 0)], 0);
    const newer = run("evalrun_new", "2026-04-20T10:00:00Z", [result("a", "passed", 1)], 1);

    render(<RunComparison evalId="eval_1" runs={[older, newer]} />);

    expect(screen.getByTitle("Improved")).toBeInTheDocument();
  });
});
