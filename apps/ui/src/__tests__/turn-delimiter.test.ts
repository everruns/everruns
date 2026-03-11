import {
  getCompletedTurnDurationsByEvent,
  formatWorkedDuration,
} from "@/components/chat/turn-delimiter";
import type { Event, EventContext } from "@/lib/api/types";

let seqCounter = 0;

function resetSeq() {
  seqCounter = 0;
}

function emptyContext(): EventContext {
  return {};
}

function turnContext(turnId: string, inputMessageId: string): EventContext {
  return { turn_id: turnId, input_message_id: inputMessageId };
}

function makeEvent(
  type: string,
  data: Record<string, unknown>,
  context: EventContext = emptyContext(),
): Event {
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

function inputMessageEvent(messageId: string, text: string, context?: EventContext): Event {
  return makeEvent(
    "input.message",
    {
      message: {
        id: messageId,
        session_id: "session-1",
        sequence: seqCounter + 1,
        role: "user",
        content: [{ type: "text", text }],
        tool_call_id: null,
        created_at: "2026-03-11T12:00:00Z",
      },
    },
    context,
  );
}

function turnStartedEvent(turnId: string, inputMessageId: string): Event {
  return makeEvent(
    "turn.started",
    { turn_id: turnId, input_message_id: inputMessageId },
    turnContext(turnId, inputMessageId),
  );
}

function outputMessageCompletedEvent(turnId: string, inputMessageId: string, text: string): Event {
  return makeEvent(
    "output.message.completed",
    {
      message: {
        id: `msg-out-${seqCounter + 1}`,
        session_id: "session-1",
        sequence: seqCounter + 1,
        role: "agent",
        content: [{ type: "text", text }],
        tool_call_id: null,
        created_at: "2026-03-11T12:00:00Z",
      },
    },
    turnContext(turnId, inputMessageId),
  );
}

function outputMessageWithClientToolCallEvent(turnId: string, inputMessageId: string): Event {
  return makeEvent(
    "output.message.completed",
    {
      message: {
        id: `msg-out-tool-${seqCounter + 1}`,
        session_id: "session-1",
        sequence: seqCounter + 1,
        role: "agent",
        content: [
          {
            type: "tool_call",
            id: "client-tool-1",
            name: "browser.prompt",
            arguments: { prompt: "Approve" },
          },
        ],
        tool_call_id: null,
        created_at: "2026-03-11T12:00:00Z",
      },
    },
    turnContext(turnId, inputMessageId),
  );
}

function toolCallRequestedEvent(turnId: string, inputMessageId: string): Event {
  return makeEvent(
    "tool.call_requested",
    {
      tool_calls: [
        {
          id: "client-tool-1",
          name: "browser.prompt",
          arguments: { prompt: "Approve" },
        },
      ],
    },
    turnContext(turnId, inputMessageId),
  );
}

function turnCompletedEvent(turnId: string, inputMessageId: string, durationMs: number): Event {
  return makeEvent(
    "turn.completed",
    { turn_id: turnId, iterations: 1, duration_ms: durationMs },
    turnContext(turnId, inputMessageId),
  );
}

describe("turn-delimiter", () => {
  beforeEach(resetSeq);

  it("attaches completed turn duration to the last visible assistant event", () => {
    const events = [
      inputMessageEvent("msg-1", "Hello"),
      turnStartedEvent("turn-1", "msg-1"),
      outputMessageCompletedEvent("turn-1", "msg-1", "Hi there"),
      turnCompletedEvent("turn-1", "msg-1", 203000),
    ];

    const durations = getCompletedTurnDurationsByEvent(events);

    expect(durations.get("evt-3")).toBe(203000);
    expect(durations.size).toBe(1);
  });

  it("uses the client tool request row when embedded tool calls are hidden", () => {
    const events = [
      inputMessageEvent("msg-1", "Need approval"),
      turnStartedEvent("turn-1", "msg-1"),
      outputMessageWithClientToolCallEvent("turn-1", "msg-1"),
      toolCallRequestedEvent("turn-1", "msg-1"),
      turnCompletedEvent("turn-1", "msg-1", 4500),
    ];

    const durations = getCompletedTurnDurationsByEvent(events);

    expect(durations.get("evt-4")).toBe(4500);
    expect(durations.has("evt-3")).toBe(false);
  });

  it("formats worked durations for the divider label", () => {
    expect(formatWorkedDuration(950)).toBe("950ms");
    expect(formatWorkedDuration(4500)).toBe("5s");
    expect(formatWorkedDuration(203000)).toBe("3m 23s");
    expect(formatWorkedDuration(3600000)).toBe("1h");
  });
});
