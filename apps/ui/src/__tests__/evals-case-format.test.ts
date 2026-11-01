import {
  draftFromCase,
  scorerSummary,
  simulationOutcome,
  updateFromDraft,
} from "@/components/evals/case-format";
import type { EvalCase, EvalCaseResult } from "@/lib/api/types";

function simulatedCase(): EvalCase {
  return {
    id: "evalcase_1",
    name: "Refund request, angry customer",
    tags: [],
    conversation: [],
    scorers: [{ type: "tool_called", tool: "create_ticket", arguments_contain: "ORD-48213" }],
    simulated_user: {
      persona: "Small shop owner, charged twice.",
      goal: "Get the duplicate charge refunded.",
      done_when: "A refund ticket exists.",
      max_turns: 6,
    },
    tool_fixtures: [{ name: "create_ticket", response: { ticket: "T-1" } }],
    timeout_seconds: 300,
    position: 0,
    created_at: "2026-11-01T00:00:00Z",
    updated_at: "2026-11-01T00:00:00Z",
  };
}

describe("eval case draft", () => {
  it("round-trips a simulated case into the update it saves as", () => {
    const draft = draftFromCase(simulatedCase());
    expect(draft.mode).toBe("simulated");
    expect(draft.maxTurns).toBe("6");
    const built = updateFromDraft(draft);
    expect("request" in built && built.request).toEqual({
      conversation: [],
      simulated_user: {
        persona: "Small shop owner, charged twice.",
        goal: "Get the duplicate charge refunded.",
        done_when: "A refund ticket exists.",
        max_turns: 6,
        model_id: null,
        virtual_user_id: null,
      },
      tool_fixtures: [{ name: "create_ticket", description: null, response: { ticket: "T-1" } }],
      timeout_seconds: 300,
    });
  });

  it("switching to scripted clears the simulator and needs a message", () => {
    const draft = { ...draftFromCase(simulatedCase()), mode: "scripted" as const };
    expect(updateFromDraft(draft)).toEqual({ error: { field: "messages" } });
    const built = updateFromDraft({ ...draft, messages: "I was charged twice\n\n" });
    expect("request" in built && built.request.simulated_user).toBeNull();
    expect("request" in built && built.request.conversation).toEqual([
      { content: "I was charged twice" },
    ]);
  });

  it("names the first problem", () => {
    const draft = draftFromCase(simulatedCase());
    expect(updateFromDraft({ ...draft, goal: " " })).toEqual({ error: { field: "goal" } });
    expect(updateFromDraft({ ...draft, maxTurns: "31" })).toEqual({
      error: { field: "maxTurns" },
    });
    const badName = {
      ...draft,
      fixtures: [{ name: "billing.lookup", description: "", response: "{}" }],
    };
    expect(updateFromDraft(badName)).toEqual({
      error: { field: "fixture", index: 0, reason: "name" },
    });
    const badJson = { ...draft, fixtures: [{ name: "lookup", description: "", response: "{" }] };
    expect(updateFromDraft(badJson)).toEqual({
      error: { field: "fixture", index: 0, reason: "response" },
    });
  });
});

describe("grader summaries and simulation outcome", () => {
  it("shows the arguments a tool call must carry", () => {
    expect(scorerSummary(simulatedCase().scorers[0])).toEqual({
      template: "{tool} called",
      params: { tool: "create_ticket" },
      kind: "rule",
      detail: "ORD-48213",
    });
    expect(scorerSummary({ type: "decision", question: "Did it stay polite?" }).kind).toBe(
      "decision",
    );
  });

  it("reads how a simulated conversation ended", () => {
    const result = {
      metadata: { simulated_user: { model: "gpt-6-luna", turns: 3, ended: "done" } },
    } as unknown as EvalCaseResult;
    expect(simulationOutcome(result)).toEqual({ model: "gpt-6-luna", turns: 3, ended: "done" });
    expect(simulationOutcome({ metadata: {} } as unknown as EvalCaseResult)).toBeNull();
  });
});
