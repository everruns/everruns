// Eval cases, their graders, and the multistep parts: a simulated user and
// tool fixtures. Hand-maintained, matching the server's eval case contract.

import type { EvalTarget } from "./legacy-api-types";

export interface EvalCase {
  id: string;
  name: string;
  description?: string;
  target?: EvalTarget;
  tags: string[];
  conversation: EvalInputMessage[];
  post?: EvalInputMessage[];
  artifacts?: ArtifactSpec[];
  scorers: Scorer[];
  max_turns?: number;
  timeout_seconds?: number;
  /** A model plays the user after the scripted messages. */
  simulated_user?: SimulatedUser | null;
  /** Tools given to the agent with canned responses. */
  tool_fixtures?: ToolFixture[] | null;
  position: number;
  created_at: string;
  updated_at: string;
}

export interface EvalInputMessage {
  content: string;
}

export interface ArtifactSpec {
  name: string;
  path: string;
}

export interface SimulatedUser {
  persona: string;
  goal: string;
  done_when?: string | null;
  /** 1 to 30, default 8. */
  max_turns?: number | null;
  /** Simulator model; the org's default when unset. */
  model_id?: string | null;
  /** Plays the user as this virtual user. */
  virtual_user_id?: string | null;
}

export interface ToolFixture {
  name: string;
  description?: string | null;
  parameters?: Record<string, unknown> | null;
  response: unknown;
}

export type Scorer =
  | { type: "contains"; text: string; weight?: number }
  | { type: "not_contains"; text: string; weight?: number }
  | { type: "regex"; pattern: string; weight?: number }
  | {
      type: "tool_called";
      tool: string;
      min?: number;
      /** Only calls whose argument JSON contains this text count. */
      arguments_contain?: string;
      weight?: number;
    }
  | { type: "tool_not_called"; tool: string; weight?: number }
  | { type: "tool_call_count"; min?: number; max?: number; weight?: number }
  | { type: "turns_within"; max: number; weight?: number }
  | { type: "tokens_within"; max: number; weight?: number }
  | { type: "file_contains"; path: string; text: string; weight?: number }
  | { type: "json_schema"; schema: object; weight?: number }
  | {
      type: "judge";
      rubric: string;
      model_id?: string;
      pass_threshold?: number;
      weight?: number;
    }
  | {
      type: "decision";
      question: string;
      yes?: string;
      no?: string;
      pass_threshold?: number;
      weight?: number;
    };

export interface CreateEvalCaseRequest {
  name: string;
  description?: string;
  target?: EvalTarget;
  tags?: string[];
  conversation?: EvalInputMessage[];
  artifacts?: ArtifactSpec[];
  scorers: Scorer[];
  max_turns?: number;
  timeout_seconds?: number;
  position?: number;
  simulated_user?: SimulatedUser;
  tool_fixtures?: ToolFixture[];
}

export interface CreateEvalRunRequest {
  target?: EvalTarget;
  model_override?: string;
  trials?: number;
  max_cost_usd?: number;
  /** Run only these cases ("Try once"). */
  case_ids?: string[];
}
