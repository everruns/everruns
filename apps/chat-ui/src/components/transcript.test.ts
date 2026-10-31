import { test } from "node:test";
import assert from "node:assert/strict";
import { artifactPath } from "./artifact-path";
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
test("a sent message is the reply and its tool call is not work", () => {
  const events: ChatEvent[] = [
    {
      type: "input.message",
      data: { message: { id: "u", role: "user", content: [{ type: "text", text: "What is this?" }] } },
    },
    {
      type: "tool.started",
      data: {
        tool_call: { id: "send", name: "send_message" },
        narration: "Sending message: It's the **About** panel.",
      },
    },
    {
      type: "tool.started",
      data: { tool_call: { id: "read", name: "read_file" }, narration: "Reading the page" },
    },
    {
      type: "conversation.message",
      data: { message_id: "sent", text: "It's the **About** panel for Bashkit." },
    },
    { type: "tool.completed", data: { tool_call_id: "send", tool_name: "send_message" } },
  ];
  assert.deepEqual(
    conversationMessages(events).map((message) => [message.role, message.text]),
    [
      ["user", "What is this?"],
      ["assistant", "It's the **About** panel for Bashkit."],
    ],
  );
  const { logs, pending } = workForMessages(events);
  assert.deepEqual(
    logs.get("sent")?.map((event) => (event.data.tool_call as { name: string }).name),
    ["read_file"],
  );
  assert.equal(pending.length, 1);
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

test("artifact URLs decode file names once and stay on workspace mounts", () => {
  assert.equal(artifactPath("/workspace/my%20report.md#summary"), "/workspace/my report.md");
  assert.equal(
    artifactPath("/attachments/%E6%8A%A5%E5%91%8A.csv?download=1"),
    "/attachments/报告.csv",
  );
  assert.equal(artifactPath("/workspace/literal%2520.md"), "/workspace/literal%20.md");
  assert.equal(artifactPath("https://example.test/workspace/report.md"), null);
  assert.equal(artifactPath("/workspace/../memory/user"), null);
  assert.equal(artifactPath("/workspace/bad%escape.md"), null);
});
