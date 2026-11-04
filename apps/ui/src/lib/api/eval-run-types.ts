// Eval run status and summaries, shared by datasets, runs and experiments.

export interface EvalRunSummaryView {
  id: string;
  status: EvalRunStatus;
  summary?: RunSummary;
  trials?: number;
  created_at: string;
}

export type EvalRunStatus = "pending" | "running" | "completed" | "failed" | "cancelled";

export interface RunSummary {
  total: number;
  passed: number;
  failed: number;
  errored: number;
  pass_rate: number;
  avg_score: number;
  avg_turns: number;
  avg_latency_ms: number;
  total_input_tokens: number;
  total_output_tokens: number;
  /** Trials per case; cases and the pass^k / pass@k counts below. */
  trials?: number;
  cases?: number;
  cases_passed_any_trial?: number;
  cases_passed_every_trial?: number;
  total_cost_usd?: number | null;
}
