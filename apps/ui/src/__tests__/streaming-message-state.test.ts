import type { Event } from "@/lib/api/types";
import { latestStreamingMessage } from "@/lib/streaming-message-state";

function event(type: string, data: Record<string, unknown>): Event {
  return {
    id: `${type}-${String(data.message_id ?? "turn")}`,
    type,
    ts: "2026-07-18T00:00:00Z",
    session_id: "session-1",
    context: { turn_id: "turn-1" },
    data,
  };
}

describe("latestStreamingMessage", () => {
  it("groups accumulated text by message_id within one turn", () => {
    const events = [
      event("output.message.started", {
        turn_id: "turn-1",
        message_id: "commentary",
        iteration: 1,
      }),
      event("output.message.delta", {
        turn_id: "turn-1",
        message_id: "commentary",
        delta: "Checking",
        accumulated: "Checking",
      }),
      event("output.message.completed", {
        message: { id: "commentary", role: "agent", content: [] },
      }),
      event("output.message.started", {
        turn_id: "turn-1",
        message_id: "final",
        iteration: 2,
      }),
      event("output.message.delta", {
        turn_id: "turn-1",
        message_id: "final",
        delta: "Done",
        accumulated: "Done",
      }),
    ];

    expect(latestStreamingMessage(events)).toEqual({
      messageId: "final",
      turnId: "turn-1",
      text: "Done",
      isThinking: false,
      iteration: 2,
    });
  });

  it("applies guardrail replacement only to its message lifecycle", () => {
    const events = [
      event("output.message.started", {
        turn_id: "turn-1",
        message_id: "message-1",
      }),
      event("output.message.delta", {
        turn_id: "turn-1",
        message_id: "message-1",
        delta: "unsafe",
        accumulated: "unsafe",
      }),
      event("output.message.replaced", {
        turn_id: "turn-1",
        message_id: "message-1",
        guardrail_capability_id: "guardrail",
        guardrail_id: "prompt-canary",
        reason_code: "blocked",
        replacement: "Safe replacement",
      }),
    ];

    expect(latestStreamingMessage(events).text).toBe("Safe replacement");
    expect(latestStreamingMessage(events).messageId).toBe("message-1");
  });
});
