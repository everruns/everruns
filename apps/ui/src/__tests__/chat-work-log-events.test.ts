import {
  getReasoningMultiIterationTurnIds,
  shouldRenderWorkLogEvent,
  isStructuralWorkLogEvent,
} from "@/components/chat/chat-work-log-events";
import type { Event, EventContext } from "@/lib/api/types";

let seqCounter = 0;

function makeEvent(type: string, data: Record<string, unknown>, context: EventContext = {}): Event {
  seqCounter++;
  return {
    id: `evt-${seqCounter}`,
    type,
    ts: `2026-03-11T12:00:${String(seqCounter).padStart(2, "0")}Z`,
    session_id: "session-1",
    context,
    data,
    sequence: seqCounter,
  };
}

function reasonCompletedEvent(turnId: string): Event {
  return makeEvent(
    "reason.completed",
    {
      success: true,
      text_preview: "A concise answer preview",
      has_tool_calls: false,
      tool_call_count: 0,
    },
    { turn_id: turnId },
  );
}

function reasonItemEvent(turnId: string): Event {
  return makeEvent("reason.item", { turn_id: turnId, summary: ["Checked the repository"] });
}

describe("chat work log event eligibility", () => {
  beforeEach(() => {
    seqCounter = 0;
  });

  it("does not render one-iteration reasoning summaries as work logs", () => {
    const event = reasonCompletedEvent("turn-1");

    expect(shouldRenderWorkLogEvent(event, new Map([["turn-1", 1]]), new Set(), new Set())).toBe(
      false,
    );
  });

  it("renders multi-iteration reasoning summaries as work logs", () => {
    const event = reasonCompletedEvent("turn-1");

    expect(shouldRenderWorkLogEvent(event, new Map([["turn-1", 2]]), new Set(), new Set())).toBe(
      true,
    );
  });

  it("keeps active tool and action turns eligible before completion arrives", () => {
    const toolRequest = makeEvent(
      "tool.call_requested",
      {
        tool_calls: [{ id: "tool-1", name: "bash", arguments: {} }],
      },
      { turn_id: "turn-1" },
    );
    const reasonItem = reasonItemEvent("turn-1");

    expect(isStructuralWorkLogEvent(toolRequest)).toBe(true);
    expect(shouldRenderWorkLogEvent(reasonItem, new Map(), new Set(["turn-1"]), new Set())).toBe(
      true,
    );
  });

  it("renders active multi-iteration reasoning before turn completion arrives", () => {
    const firstIteration = reasonCompletedEvent("turn-1");
    const secondIteration = reasonCompletedEvent("turn-1");
    const multiIterationTurnIds = getReasoningMultiIterationTurnIds([
      firstIteration,
      secondIteration,
    ]);

    expect(multiIterationTurnIds.has("turn-1")).toBe(true);
    expect(
      shouldRenderWorkLogEvent(firstIteration, new Map(), new Set(), multiIterationTurnIds),
    ).toBe(true);
  });
});
