// Pure helpers for the eval case page (approved mockup 05): the editable
// draft of a case, the update request it saves as, and grader summaries.

import type {
  EvalCase,
  EvalCaseResult,
  Scorer,
  SimulatedUser,
  ToolFixture,
  UpdateEvalCaseRequest,
} from "@/lib/api/types";

export type ConversationMode = "scripted" | "simulated";

/** Fixture as edited: the response is JSON text until it is saved. */
export interface FixtureDraft {
  name: string;
  description: string;
  response: string;
}

export interface CaseDraft {
  mode: ConversationMode;
  /** Scripted messages, one per line. */
  messages: string;
  persona: string;
  goal: string;
  doneWhen: string;
  maxTurns: string;
  modelId: string;
  virtualUserId: string;
  fixtures: FixtureDraft[];
  timeoutSeconds: string;
}

export const DEFAULT_SIMULATED_TURNS = 8;
export const MAX_SIMULATED_TURNS = 30;

export function caseMode(evalCase: Pick<EvalCase, "simulated_user">): ConversationMode {
  return evalCase.simulated_user ? "simulated" : "scripted";
}

export function draftFromCase(evalCase: EvalCase): CaseDraft {
  const user = evalCase.simulated_user;
  return {
    mode: caseMode(evalCase),
    messages: evalCase.conversation.map((m) => m.content).join("\n"),
    persona: user?.persona ?? "",
    goal: user?.goal ?? "",
    doneWhen: user?.done_when ?? "",
    maxTurns: String(user?.max_turns ?? DEFAULT_SIMULATED_TURNS),
    modelId: user?.model_id ?? "",
    virtualUserId: user?.virtual_user_id ?? "",
    fixtures: (evalCase.tool_fixtures ?? []).map((f) => ({
      name: f.name,
      description: f.description ?? "",
      response: JSON.stringify(f.response, null, 2),
    })),
    timeoutSeconds: evalCase.timeout_seconds ? String(evalCase.timeout_seconds) : "",
  };
}

export type DraftError =
  | { field: "persona" | "goal" | "messages" | "maxTurns" | "timeoutSeconds" }
  | { field: "fixture"; index: number; reason: "name" | "response" };

const FIXTURE_NAME = /^[A-Za-z0-9_-]{1,64}$/;

/**
 * The PATCH a draft saves as, or the first problem with it. Scripted mode
 * clears the simulated user; scripted messages stay in both modes, since a
 * simulated case may open with fixed messages.
 */
export function updateFromDraft(
  draft: CaseDraft,
): { request: UpdateEvalCaseRequest } | { error: DraftError } {
  const conversation = draft.messages
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((content) => ({ content }));

  let simulated: SimulatedUser | null = null;
  if (draft.mode === "simulated") {
    if (!draft.persona.trim()) return { error: { field: "persona" } };
    if (!draft.goal.trim()) return { error: { field: "goal" } };
    const turns = Number(draft.maxTurns);
    if (!Number.isInteger(turns) || turns < 1 || turns > MAX_SIMULATED_TURNS) {
      return { error: { field: "maxTurns" } };
    }
    simulated = {
      persona: draft.persona.trim(),
      goal: draft.goal.trim(),
      done_when: draft.doneWhen.trim() || null,
      max_turns: turns,
      model_id: draft.modelId || null,
      virtual_user_id: draft.virtualUserId || null,
    };
  } else if (conversation.length === 0) {
    return { error: { field: "messages" } };
  }

  const fixtures: ToolFixture[] = [];
  for (const [index, f] of draft.fixtures.entries()) {
    if (!FIXTURE_NAME.test(f.name.trim()) || f.name.trim().startsWith("mcp_")) {
      return { error: { field: "fixture", index, reason: "name" } };
    }
    let response: unknown;
    try {
      response = JSON.parse(f.response);
    } catch {
      return { error: { field: "fixture", index, reason: "response" } };
    }
    fixtures.push({
      name: f.name.trim(),
      description: f.description.trim() || null,
      response,
    });
  }

  let timeout: number | undefined;
  if (draft.timeoutSeconds.trim()) {
    timeout = Number(draft.timeoutSeconds);
    if (!Number.isInteger(timeout) || timeout < 1) return { error: { field: "timeoutSeconds" } };
  }

  return {
    request: {
      conversation,
      simulated_user: simulated,
      tool_fixtures: fixtures.length > 0 ? fixtures : null,
      ...(timeout !== undefined ? { timeout_seconds: timeout } : {}),
    },
  };
}

export type ScorerKind = "rule" | "judge" | "decision" | "metric";

/**
 * What a grader checks: a translatable template with its values, the kind
 * of check, and an optional detail line (the text a tool call's arguments
 * must contain).
 */
export interface ScorerSummary {
  template: string;
  params: Record<string, string | number>;
  kind: ScorerKind;
  detail?: string;
}

export function scorerSummary(scorer: Scorer): ScorerSummary {
  switch (scorer.type) {
    case "contains":
      return { template: "Answer contains “{text}”", params: { text: scorer.text }, kind: "rule" };
    case "not_contains":
      return {
        template: "Answer does not contain “{text}”",
        params: { text: scorer.text },
        kind: "rule",
      };
    case "regex":
      return {
        template: "Answer matches /{pattern}/",
        params: { pattern: scorer.pattern },
        kind: "rule",
      };
    case "tool_called":
      return {
        template: "{tool} called",
        params: { tool: scorer.tool },
        kind: "rule",
        detail: scorer.arguments_contain,
      };
    case "tool_not_called":
      return { template: "{tool} not called", params: { tool: scorer.tool }, kind: "rule" };
    case "tool_call_count":
      return {
        template: "{min} to {max} tool calls",
        params: { min: scorer.min ?? 0, max: scorer.max ?? "∞" },
        kind: "metric",
      };
    case "turns_within":
      return { template: "At most {max} turns", params: { max: scorer.max }, kind: "metric" };
    case "tokens_within":
      return { template: "At most {max} tokens", params: { max: scorer.max }, kind: "metric" };
    case "file_contains":
      return {
        template: "{path} contains “{text}”",
        params: { path: scorer.path, text: scorer.text },
        kind: "rule",
      };
    case "json_schema":
      return { template: "Answer matches a JSON schema", params: {}, kind: "rule" };
    case "judge":
      return { template: "{text}", params: { text: scorer.rubric }, kind: "judge" };
    case "decision":
      return { template: "{text}", params: { text: scorer.question }, kind: "decision" };
    default:
      return {
        template: "{text}",
        params: { text: (scorer as { type: string }).type },
        kind: "rule",
      };
  }
}

/** How a simulated conversation ended, from a result's metadata. */
export interface SimulationOutcome {
  model: string | null;
  turns: number;
  ended: "done" | "turn_limit";
}

export function simulationOutcome(result: EvalCaseResult): SimulationOutcome | null {
  const raw = (result.metadata as Record<string, unknown> | null | undefined)?.simulated_user;
  if (!raw || typeof raw !== "object") return null;
  const value = raw as Record<string, unknown>;
  return {
    model: typeof value.model === "string" ? value.model : null,
    turns: typeof value.turns === "number" ? value.turns : 0,
    ended: value.ended === "done" ? "done" : "turn_limit",
  };
}
