// Eval domain types matching Rust Eval, EvalCase, EvalRun, EvalCaseResult

export interface Eval {
  id: string;
  name: string;
  description?: string;
  agent_id: string;
  harness_id: string;
  model_override?: string;
  tags: string[];
  status: "active" | "archived" | "deleted";
  case_count: number;
  last_run?: EvalRunSummaryView;
  created_at: string;
  updated_at: string;
}

export interface EvalRunSummaryView {
  id: string;
  status: EvalRunStatus;
  summary?: RunSummary;
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
}

export interface EvalCase {
  id: string;
  name: string;
  description?: string;
  tags: string[];
  conversation: EvalInputMessage[];
  scorers: Scorer[];
  max_turns?: number;
  timeout_seconds?: number;
  position: number;
  created_at: string;
  updated_at: string;
}

export interface EvalInputMessage {
  content: string;
}

export type Scorer =
  | { type: "contains"; text: string; weight?: number }
  | { type: "not_contains"; text: string; weight?: number }
  | { type: "regex"; pattern: string; weight?: number }
  | { type: "tool_called"; tool: string; min?: number; weight?: number }
  | { type: "tool_not_called"; tool: string; weight?: number }
  | { type: "tool_call_count"; min?: number; max?: number; weight?: number }
  | { type: "turns_within"; max: number; weight?: number }
  | { type: "file_contains"; path: string; text: string; weight?: number }
  | { type: "json_schema"; schema: object; weight?: number };

export interface EvalRun {
  id: string;
  model_override?: string;
  filter_tags?: string[];
  status: EvalRunStatus;
  triggered_by: string;
  started_at?: string;
  completed_at?: string;
  summary?: RunSummary;
  results: EvalCaseResult[];
  created_at: string;
  updated_at: string;
}

export interface EvalCaseResult {
  id: string;
  eval_case_id: string;
  case_name?: string;
  session_id?: string;
  status: "pending" | "running" | "passed" | "failed" | "errored" | "timeout";
  scores?: Record<string, { pass: boolean; value: number; reason: string }>;
  turns?: number;
  latency_ms?: number;
  input_tokens?: number;
  output_tokens?: number;
  error_message?: string;
  created_at: string;
  updated_at: string;
}

// Request types
export interface CreateEvalRequest {
  name: string;
  description?: string;
  agent_id: string;
  harness_id: string;
  model_override?: string;
  tags?: string[];
}

export interface UpdateEvalRequest {
  name?: string;
  description?: string;
  agent_id?: string;
  harness_id?: string;
  model_override?: string;
  tags?: string[];
}

export interface CreateEvalCaseRequest {
  name: string;
  description?: string;
  tags?: string[];
  conversation: EvalInputMessage[];
  scorers: Scorer[];
  max_turns?: number;
  timeout_seconds?: number;
  position?: number;
}

export interface CreateEvalRunRequest {
  model_override?: string;
  filter_tags?: string[];
}
