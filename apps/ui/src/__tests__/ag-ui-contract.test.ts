import { EventSchemas, RunAgentInputSchema } from "@ag-ui/core";

describe("AG-UI contract fixtures", () => {
  const threadId = "00000000-0000-0000-0000-000000000001";
  const runId = "00000000-0000-0000-0000-000000000002";
  const messageId = "00000000-0000-0000-0000-000000000003";

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

  it("validates representative AG-UI events we emit", () => {
    const events = [
      { type: "RUN_STARTED", threadId, runId },
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
      { type: "THINKING_START" },
      { type: "THINKING_TEXT_MESSAGE_START" },
      { type: "THINKING_TEXT_MESSAGE_CONTENT", delta: "planning" },
      { type: "THINKING_TEXT_MESSAGE_END" },
      { type: "THINKING_END" },
      { type: "TEXT_MESSAGE_START", messageId, role: "assistant" },
      { type: "TEXT_MESSAGE_CONTENT", messageId, delta: "Hello" },
      { type: "TEXT_MESSAGE_END", messageId },
      {
        type: "TOOL_CALL_START",
        toolCallId: "call_12345678",
        toolCallName: "search",
        parentMessageId: messageId,
      },
      { type: "TOOL_CALL_ARGS", toolCallId: "call_12345678", delta: '{"q":"hi"}' },
      { type: "TOOL_CALL_END", toolCallId: "call_12345678" },
      {
        type: "TOOL_CALL_RESULT",
        messageId,
        toolCallId: "call_12345678",
        content: "done",
        role: "tool",
      },
      { type: "RUN_FINISHED", threadId, runId },
      { type: "RUN_ERROR", message: "Run failed" },
    ];

    for (const event of events) {
      expect(() => EventSchemas.parse(event)).not.toThrow();
    }
  });
});
