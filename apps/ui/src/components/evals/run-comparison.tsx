"use client";

// Cross-run comparison: a per-case × run grid. Runs are ordered oldest →
// newest so a cell can be compared to the same case in the previous run and
// flag regressions (passed → failed) and improvements (failed → passed). With a
// pass-rate row on top, this is the "did this run regress vs last" view.

import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { meanScore } from "@/components/evals/eval-format";
import type { EvalCaseResult, EvalRun } from "@/lib/api/types";

type Verdict = "passed" | "failed" | "skipped" | "none";

interface CaseAgg {
  verdict: Verdict;
  score: number | null;
}

// A case can have several results in one run (the matrix). Roll them up to a
// single per-case verdict/score for the comparison cell.
function aggregateCase(results: EvalCaseResult[]): CaseAgg {
  if (results.length === 0) return { verdict: "none", score: null };
  const statuses = results.map((r) => r.status);
  let verdict: Verdict;
  if (statuses.every((s) => s === "passed")) {
    verdict = "passed";
  } else if (statuses.some((s) => s === "failed" || s === "errored" || s === "timeout")) {
    verdict = "failed";
  } else {
    verdict = "skipped";
  }
  const scores = results.map((r) => meanScore(r.scores)).filter((v): v is number => v != null);
  const score = scores.length > 0 ? scores.reduce((a, v) => a + v, 0) / scores.length : null;
  return { verdict, score };
}

function caseKeyOf(r: EvalCaseResult): string {
  return r.case_name ?? r.eval_case_id;
}

function verdictClasses(verdict: Verdict): string {
  switch (verdict) {
    case "passed":
      return "bg-green-500/15 text-green-700 dark:text-green-400";
    case "failed":
      return "bg-red-500/15 text-red-700 dark:text-red-400";
    case "skipped":
      return "bg-muted text-muted-foreground";
    default:
      return "text-muted-foreground/50";
  }
}

function runShortId(run: EvalRun): string {
  // Public ids are `evalrun_<32 hex>`; the last 6 are enough to disambiguate.
  return run.id.replace(/^evalrun_/, "").slice(-6);
}

export function RunComparison({ evalId, runs }: { evalId: string; runs: EvalRun[] }) {
  // Oldest → newest so "previous run" is the column to the left.
  const ordered = [...runs].sort((a, b) => a.created_at.localeCompare(b.created_at));

  // Union of case keys, in first-seen order across runs.
  const caseKeys: string[] = [];
  // perRun[i] maps caseKey -> aggregate for run i.
  const perRun: Array<Map<string, CaseAgg>> = ordered.map((run) => {
    const byCase = new Map<string, EvalCaseResult[]>();
    for (const r of run.results) {
      const key = caseKeyOf(r);
      if (!caseKeys.includes(key)) caseKeys.push(key);
      const list = byCase.get(key) ?? [];
      list.push(r);
      byCase.set(key, list);
    }
    const aggs = new Map<string, CaseAgg>();
    for (const [key, list] of byCase) aggs.set(key, aggregateCase(list));
    return aggs;
  });

  return (
    <div className="space-y-3">
      <div className="overflow-x-auto">
        <table className="w-full border-separate border-spacing-1">
          <thead>
            <tr>
              <th className="sticky left-0 z-10 bg-background py-2 px-3 text-left text-sm font-medium text-muted-foreground">
                Case
              </th>
              {ordered.map((run) => (
                <th key={run.id} className="py-2 px-3 text-center align-bottom">
                  <Link
                    href={`/evals/${evalId}/runs/${run.id}`}
                    className="text-xs font-medium hover:underline"
                  >
                    {runShortId(run)}
                  </Link>
                  <div className="text-[10px] font-normal text-muted-foreground whitespace-nowrap">
                    {new Date(run.created_at).toLocaleDateString()}
                  </div>
                  {run.source === "external" && (
                    <Badge variant="secondary" className="mt-1 text-[10px]">
                      {run.attribution?.system ?? "external"}
                    </Badge>
                  )}
                </th>
              ))}
            </tr>
            <tr>
              <th className="sticky left-0 z-10 bg-background py-1 px-3 text-left text-xs font-normal text-muted-foreground">
                Pass rate
              </th>
              {ordered.map((run) => (
                <th
                  key={run.id}
                  className="py-1 px-3 text-center text-xs font-medium text-muted-foreground"
                >
                  {run.summary ? `${(run.summary.pass_rate * 100).toFixed(0)}%` : "—"}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {caseKeys.map((caseKey) => (
              <tr key={caseKey}>
                <td className="sticky left-0 z-10 bg-background py-2 px-3 text-sm font-medium whitespace-nowrap">
                  {caseKey}
                </td>
                {ordered.map((run, i) => {
                  const agg = perRun[i].get(caseKey);
                  const prev = i > 0 ? perRun[i - 1].get(caseKey) : undefined;
                  const regressed = prev?.verdict === "passed" && agg?.verdict === "failed";
                  const improved = prev?.verdict === "failed" && agg?.verdict === "passed";
                  const ring = regressed
                    ? "ring-2 ring-red-500"
                    : improved
                      ? "ring-2 ring-green-500"
                      : "";
                  const text =
                    agg == null
                      ? "·"
                      : agg.score != null
                        ? agg.score.toFixed(2)
                        : agg.verdict === "passed"
                          ? "✓"
                          : agg.verdict === "failed"
                            ? "✗"
                            : "–";
                  return (
                    <td
                      key={run.id}
                      title={
                        regressed
                          ? "Regressed from previous run"
                          : improved
                            ? "Improved"
                            : undefined
                      }
                      className={`py-2 px-3 text-center text-sm font-medium rounded-md ${verdictClasses(agg?.verdict ?? "none")} ${ring}`}
                    >
                      {text}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex items-center gap-4 text-xs text-muted-foreground">
        <span className="flex items-center gap-1">
          <span className="inline-block w-3 h-3 rounded ring-2 ring-red-500" /> regressed
        </span>
        <span className="flex items-center gap-1">
          <span className="inline-block w-3 h-3 rounded ring-2 ring-green-500" /> improved
        </span>
        <span>cells show mean score (✓/✗ when no numeric score); runs ordered oldest → newest</span>
      </div>
    </div>
  );
}
