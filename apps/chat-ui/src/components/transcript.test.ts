import { test } from "node:test";
import assert from "node:assert/strict";
import { conversationMessages, type ChatEvent } from "./transcript";
import { workForMessages } from "./work-log";
test("completed messages replace streamed deltas and keep tool results out of the transcript", () => {
  const events: ChatEvent[] = [
    {
      type: "input.message",
      data: {
        message: { id: "u", role: "user", content: [{ type: "text", text: "Make a report" }] },
      },
    },
    { type: "tool.completed", data: { tool_call_id: "t", result: "private tool output" } },
    { type: "output.message.delta", data: { message_id: "a", delta: "Partial" } },
    {
      type: "output.message.completed",
      data: {
        message: { id: "a", role: "agent", content: [{ type: "text", text: "Final report" }] },
      },
    },
  ];
  assert.deepEqual(
    conversationMessages(events).map((message) => message.text),
    ["Make a report", "Final report"],
  );
});
test("each reply receives the work from its own user turn", () => {
  const events: ChatEvent[] = [
    { type: "input.message", data: { message: { id: "u1" } } },
    { type: "tool.started", data: { tool_call: { id: "tool1", name: "read_file" } } },
    { type: "output.message.completed", data: { message: { id: "a1" } } },
    { type: "input.message", data: { message: { id: "u2" } } },
    { type: "tool.started", data: { tool_call: { id: "tool2", name: "write_file" } } },
    { type: "output.message.delta", data: { message_id: "a2", delta: "Working" } },
  ];
  const { logs } = workForMessages(events);
  assert.equal(logs.get("a1")?.length, 1);
  assert.equal((logs.get("a2")?.[0].data.tool_call as { id: string }).id, "tool2");
});
