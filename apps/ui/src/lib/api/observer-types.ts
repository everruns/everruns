// ============================================
// Observer types (online scoring of production sessions)
// ============================================
// Hand-written (not generated): the backend exposes a flattened tagged union for
// scorer configs and a snake_case tagged union for rule scorers. See the
// `/v1/observers` API contract.

export type ObserverStatus = "active" | "paused" | "archived";

/** Match rules deciding which production turns an observer samples. */
export interface ObserverMatch {
  agent_ids?: string[];
  harness_ids?: string[];
  session_tags?: string[];
}

/**
 * Deterministic rule scorer variants (snake_case, discriminated by `type`).
 * Mirrors the eval Scorer union, minus `file_contains` (rejected by the server
 * for observers). `weight` defaults to 1.0 server-side.
 */
export type ScorerRule =
  | { type: "contains"; text: string; weight?: number }
  | { type: "not_contains"; text: string; weight?: number }
  | { type: "regex"; pattern: string; weight?: number }
  | { type: "tool_called"; tool: string; min?: number; weight?: number }
  | { type: "tool_not_called"; tool: string; weight?: number }
  | { type: "tool_call_count"; min?: number; max?: number; weight?: number }
  | { type: "turns_within"; max: number; weight?: number }
  | { type: "json_schema"; schema: object; weight?: number };

/** Trace slice a scorer grades: one turn, the whole conversation, or one tool call. */
export type ObserverScorerScope = "turn" | "session" | "tool";

/**
 * Per-scorer configuration. FLATTENED tagged union discriminated by `method`:
 * the method-specific fields sit alongside `key`/`scope`, not nested.
 */
export type ObserverScorerConfig =
  | {
      key: string;
      scope: ObserverScorerScope;
      /** Tool scope only: grade calls to these tools; empty grades every call. */
      tools?: string[];
      method: "rule";
      rule: ScorerRule;
    }
  | {
      key: string;
      scope: ObserverScorerScope;
      tools?: string[];
      method: "llm_judge";
      rubric: string;
      model_id?: string;
      pass_threshold: number;
    };

export interface Observer {
  id: string;
  name: string;
  description?: string;
  match: ObserverMatch;
  sampling_rate: number;
  scorers: ObserverScorerConfig[];
  alert?: ObserverAlert;
  status: ObserverStatus;
  created_at: string;
  updated_at: string;
  archived_at?: string;
}

/** Pass-rate alert: opens a health issue when the rate falls under `min_pass_rate`. */
export interface ObserverAlert {
  min_pass_rate: number;
  window_hours?: number;
  min_scored?: number;
}

export interface CreateObserverRequest {
  name: string;
  description?: string;
  match?: ObserverMatch;
  sampling_rate?: number;
  scorers: ObserverScorerConfig[];
  alert?: ObserverAlert;
}

export interface UpdateObserverRequest {
  name?: string;
  description?: string;
  match?: ObserverMatch;
  sampling_rate?: number;
  scorers?: ObserverScorerConfig[];
  status?: ObserverStatus;
  /** `null` removes the alert. */
  alert?: ObserverAlert | null;
}

export type TraceScoreStatus = "pending" | "scoring" | "completed" | "errored" | "skipped";

/** A person's verdict on one score. */
export interface ScoreReview {
  agrees: boolean;
  note?: string;
  reviewed_by?: string;
  reviewed_at?: string;
}

/** One scored trace slice (turn, session or tool call) produced by an observer. */
export interface TraceScore {
  id: string;
  observer_id: string;
  scorer_key: string;
  session_id: string;
  scope?: ObserverScorerScope;
  /** The turn id, `session`, or the tool call id, by scope. */
  turn_id: string;
  agent_id?: string;
  harness_id?: string;
  status: TraceScoreStatus;
  pass?: boolean;
  value?: number;
  label?: string;
  reason?: string;
  judge_input_tokens?: number;
  judge_output_tokens?: number;
  judge_cost_usd?: number;
  error_message?: string;
  review?: ScoreReview;
  created_at: string;
  updated_at: string;
}

/** Completed observer scores in one group of a live-quality summary. */
export interface QualityGroup {
  key: string;
  scored: number;
  passed: number;
  pass_rate?: number;
  sessions: number;
  /** Daily pass rates, oldest first (agent groups only). */
  trend?: number[];
}

/** How production traffic scored over the last `days`. */
export interface QualitySummary {
  since: string;
  days: number;
  scored: number;
  passed: number;
  pass_rate?: number;
  sessions: number;
  by_agent: QualityGroup[];
  by_observer: QualityGroup[];
  by_day: QualityGroup[];
}
