import { EventSchemas, RunAgentInputSchema } from "@ag-ui/core/schemas";

describe("upstream AG-UI schema compatibility fixtures", () => {
  const threadId = "00000000-0000-0000-0000-000000000001";
  const runId = "00000000-0000-0000-0000-000000000002";
  const messageId = "00000000-0000-0000-0000-000000000003";

  it("validates a 1.0 run request with a protocol version", () => {
    const payload = {
      threadId,
      runId,
      protocolVersion: "1.0",
      messages: [
        {
          id: "00000000-0000-0000-0000-000000000005",
          role: "user",
          content: "Hi",
        },
      ],
    };

    expect(() => RunAgentInputSchema.parse(payload)).not.toThrow();
  });

  it("validates a canonical run request", () => {
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

  it("accepts the supported event vocabulary in compatibility fixtures", () => {
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
      { type: "REASONING_START", messageId: `${runId}-reasoning-1` },
      {
        type: "REASONING_MESSAGE_START",
        messageId: `${runId}-reasoning-message-2`,
        role: "reasoning",
      },
      {
        type: "REASONING_MESSAGE_CONTENT",
        messageId: `${runId}-reasoning-message-2`,
        delta: "planning",
      },
      {
        type: "REASONING_MESSAGE_END",
        messageId: `${runId}-reasoning-message-2`,
      },
      { type: "REASONING_END", messageId: `${runId}-reasoning-1` },
      { type: "TEXT_MESSAGE_START", messageId, role: "assistant" },
      { type: "TEXT_MESSAGE_CONTENT", messageId, delta: "Hello" },
      { type: "TEXT_MESSAGE_END", messageId },
      {
        type: "TOOL_CALL_START",
        toolCallId: "call_12345678",
        toolCallName: "search",
        parentMessageId: messageId,
      },
      {
        type: "TOOL_CALL_ARGS",
        toolCallId: "call_12345678",
        delta: '{"q":"hi"}',
      },
      { type: "TOOL_CALL_END", toolCallId: "call_12345678" },
      {
        type: "TOOL_CALL_RESULT",
        messageId,
        toolCallId: "call_12345678",
        content: "done",
        role: "tool",
      },
      { type: "RUN_FINISHED", threadId, runId },
      { type: "RUN_FINISHED", threadId, runId, outcome: { type: "cancelled" } },
      { type: "RUN_ERROR", message: "Run failed", code: "internal_error" },
    ];

    for (const event of events) {
      expect(() => EventSchemas.parse(event)).not.toThrow();
    }
  });
});
