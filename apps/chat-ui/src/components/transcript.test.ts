import { test } from "node:test";
import assert from "node:assert/strict";
import { artifactPath } from "./artifact-path";
import { conversationMessages, originLabel, type ChatEvent } from "./transcript";
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
      data: {
        message: { id: "u", role: "user", content: [{ type: "text", text: "What is this?" }] },
      },
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
test("a replay that appends an earlier turn does not merge its work into the next", () => {
  const events: ChatEvent[] = [
    {
      sequence: 4,
      type: "input.message",
      data: {
        message: { id: "u2", role: "user", content: [{ type: "text", text: "Give me a joke" }] },
      },
    },
    {
      sequence: 1,
      type: "input.message",
      data: {
        message: { id: "u1", role: "user", content: [{ type: "text", text: "What do we see?" }] },
      },
    },
    {
      sequence: 2,
      type: "tool.started",
      data: { tool_call: { id: "read1", name: "read_file" }, narration: "Reading the page" },
    },
    {
      sequence: 3,
      type: "conversation.message",
      data: { message_id: "a1", text: "It's the About panel." },
    },
    {
      sequence: 5,
      type: "tool.started",
      data: {
        tool_call: { id: "send", name: "send_message" },
        narration: "Sending message: Why don't skeletons fight?",
      },
    },
    {
      sequence: 6,
      type: "tool.completed",
      data: { tool_call_id: "send", tool_name: "send_message", display_name: "Send message" },
    },
    {
      sequence: 7,
      type: "tool.started",
      data: { tool_call: { id: "read2", name: "web_search" }, narration: "Searching" },
    },
    {
      sequence: 8,
      type: "output.message.delta",
      data: { message_id: "a1", delta: " should not steal the next turn" },
    },
  ];
  assert.deepEqual(
    conversationMessages(events).map((message) => message.id),
    ["u1", "a1", "u2"],
  );
  const { logs, byUser, pending } = workForMessages(events);
  assert.deepEqual(
    logs.get("a1")?.map((event) => (event.data.tool_call as { id: string }).id),
    ["read1"],
  );
  assert.deepEqual(
    byUser.get("u1")?.map((event) => (event.data.tool_call as { id: string }).id),
    ["read1"],
  );
  assert.deepEqual(
    byUser.get("u2")?.map((event) => (event.data.tool_call as { name: string }).name),
    ["web_search"],
  );
  assert.equal(pending.length, 1);
  assert.equal((pending[0].data.tool_call as { id: string }).id, "read2");
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
test("platform messages are marked and silent runs from one source collapse", () => {
  const scheduled = (id: string) => ({
    type: "input.message",
    data: {
      message: {
        id,
        role: "user",
        content: [{ type: "text", text: "Shipmail claim poll. Read the state file." }],
        metadata: {
          everruns_origin: { kind: "schedule", name: "Shipmail claim poll", source_id: "sched_1" },
        },
      },
    },
  });
  const events: ChatEvent[] = [
    {
      type: "input.message",
      data: {
        message: { id: "u", role: "user", content: [{ type: "text", text: "Keep checking" }] },
      },
    },
    scheduled("s1"),
    scheduled("s2"),
    scheduled("s3"),
    {
      type: "output.message.completed",
      data: { message: { id: "a", role: "agent", content: [{ type: "text", text: "Approved." }] } },
    },
    scheduled("s4"),
  ];
  const messages = conversationMessages(events);
  assert.deepEqual(
    messages.map((message) => [message.id, message.origin ? originLabel(message.origin) : null]),
    [
      ["u", null],
      ["s3", "Scheduled check · Shipmail claim poll · 3×"],
      ["a", null],
      ["s4", "Scheduled check · Shipmail claim poll"],
    ],
  );
});
