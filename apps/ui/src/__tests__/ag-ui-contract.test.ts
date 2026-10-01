// Fixtures mirror what the server endpoint accepts and emits
// (the `everruns-ag-ui` crate, pinned by crates/server/src/api/ag_ui/wire_tests.rs).
// AG-UI 1.0 moved the zod schemas to the `@ag-ui/core/schemas` subpath.
import { EventSchemas, PROTOCOL_VERSION, RunAgentInputSchema } from "@ag-ui/core/schemas";

describe("upstream AG-UI 1.0 schema compatibility fixtures", () => {
  const threadId = "00000000-0000-0000-0000-000000000001";
  const runId = "00000000-0000-0000-0000-000000000002";
  const messageId = "00000000-0000-0000-0000-000000000003";
  const reasoningSpanId = "00000000-0000-0000-0000-000000000005";
  const reasoningMessageId = "00000000-0000-0000-0000-000000000006";

  it("speaks the protocol version the server announces on RUN_STARTED", () => {
    expect(PROTOCOL_VERSION).toBe("1.0");
  });

  it("validates a canonical pre-1.0-shaped run request", () => {
    const payload = {
      threadId,
      runId,
      state: {},
      messages: [
        {
          id: "00000000-0000-0000-0000-000000000004",
          role: "user",
          content: "Hello",
        },
      ],
      tools: [],
      context: [],
      forwardedProps: {},
    };

    expect(() => RunAgentInputSchema.parse(payload)).not.toThrow();
  });

  it("validates a minimal 1.0 run request with echoed reasoning and text parts", () => {
    const payload = {
      threadId,
      runId,
      protocolVersion: "1.0",
      messages: [
        { id: "u1", role: "user", content: "Hi" },
        { id: "r1", role: "reasoning", content: "earlier reasoning" },
        { id: "a1", role: "assistant", content: "Hello" },
        {
          id: "u2",
          role: "user",
          content: [
            { type: "text", text: "Next " },
            { type: "text", text: "question" },
          ],
        },
      ],
    };

    expect(() => RunAgentInputSchema.parse(payload)).not.toThrow();
  });

  it("accepts the event vocabulary the server emits", () => {
    const events = [
      { type: "RUN_STARTED", threadId, runId, protocolVersion: "1.0" },
      {
        type: "MESSAGES_SNAPSHOT",
        messages: [
          {
            id: "00000000-0000-0000-0000-000000000010",
            role: "assistant",
            content: "Snapshot message",
          },
        ],
      },
      { type: "REASONING_START", messageId: reasoningSpanId },
      { type: "REASONING_MESSAGE_START", messageId: reasoningMessageId, role: "reasoning" },
      { type: "REASONING_MESSAGE_CONTENT", messageId: reasoningMessageId, delta: "planning" },
      { type: "REASONING_MESSAGE_END", messageId: reasoningMessageId },
      { type: "REASONING_END", messageId: reasoningSpanId },
      { type: "TEXT_MESSAGE_START", messageId, role: "assistant" },
      { type: "TEXT_MESSAGE_CONTENT", messageId, delta: "Hello" },
      { type: "TEXT_MESSAGE_END", messageId },
      { type: "RUN_FINISHED", threadId, runId },
      { type: "RUN_ERROR", message: "Run failed", code: "internal_error" },
    ];

    for (const event of events) {
      expect(() => EventSchemas.parse(event)).not.toThrow();
    }
  });

  it("rejects the retired pre-1.0 THINKING_* vocabulary", () => {
    for (const type of [
      "THINKING_START",
      "THINKING_TEXT_MESSAGE_START",
      "THINKING_TEXT_MESSAGE_CONTENT",
      "THINKING_TEXT_MESSAGE_END",
      "THINKING_END",
    ]) {
      expect(EventSchemas.safeParse({ type, delta: "planning" }).success).toBe(false);
    }
  });

  it("rejects null optionals, which the server therefore omits", () => {
    expect(
      EventSchemas.safeParse({ type: "RUN_ERROR", message: "Run failed", code: null }).success,
    ).toBe(false);
  });
});
