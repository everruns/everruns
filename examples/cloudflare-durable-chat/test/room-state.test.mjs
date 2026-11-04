import test from "node:test";
import assert from "node:assert/strict";
import {
  accept,
  deliverDue,
  emptyRoom,
  historySeed,
  legacyRoom,
} from "../worker/room-state.js";

const id = "session_0123456789abcdef0123456789abcdef";
test("command receipts survive reopening and reject key reuse without another delivery", () => {
  let data = emptyRoom();
  const command = { type: "create", request_id: "create", title: "Decisions" };
  assert.equal(accept(data, command, id).conversation, id);
  data = JSON.parse(JSON.stringify(data));
  accept(data, command, id);
  assert.equal(data.outbox.length, 1);
  assert.throws(
    () => accept(data, { ...command, title: "Other" }, id),
    /different input/,
  );
  const send = {
    type: "send",
    request_id: "send",
    conversation: id,
    username: "Alice",
    text: "PostgreSQL",
  };
  accept(data, send, id);
  accept(data, send, id);
  assert.equal(data.outbox.length, 2);
});
test("timer advancement and durable outbox commit together and duplicate alarms do not repeat", () => {
  let data = emptyRoom();
  data.schedules.once = {
    conversation: id,
    instruction: "review",
    due: 100,
    active: true,
  };
  deliverDue(data, 100);
  data = JSON.parse(JSON.stringify(data));
  deliverDue(data, 101);
  assert.equal(data.outbox.length, 1);
  assert.equal(data.outbox[0].id, "timer:once:100");
  data.schedules.repeat = {
    conversation: id,
    instruction: "review",
    due: 10,
    repeat_ms: 50,
    active: true,
  };
  deliverDue(data, 1000);
  deliverDue(data, 1000);
  assert.equal(data.outbox.length, 2);
  assert.equal(data.schedules.repeat.due, 1050);
});
test("history seed excludes tool exchanges and retains stable canonical message identities", () => {
  const events = [
    {
      type: "input.message",
      sequence: 1,
      data: {
        message: {
          id: "message_user",
          role: "user",
          content: [{ type: "text", text: "hello" }],
        },
      },
    },
    {
      type: "output.message.completed",
      sequence: 2,
      data: {
        message: {
          id: "message_call",
          role: "agent",
          content: [
            { type: "tool_call", id: "call", name: "lookup", arguments: {} },
          ],
        },
      },
    },
    {
      type: "tool.completed",
      sequence: 3,
      data: { result: [{ type: "text", text: "result" }] },
    },
    {
      type: "output.message.completed",
      sequence: 4,
      data: {
        message: {
          id: "message_reply",
          role: "agent",
          content: [{ type: "text", text: "answer" }],
        },
      },
    },
  ];
  assert.deepEqual(
    historySeed(events).map((m) => m.id),
    ["message_user", "message_reply"],
  );
  assert.deepEqual(
    historySeed(events, 1).map((m) => m.id),
    ["message_user"],
  );
});
test("legacy migration preserves stable pending message IDs and receipt fingerprints", () => {
  const message = {
    id: "message_0123456789abcdef0123456789abcdef",
    role: "user",
    content: [{ type: "text", text: "queued" }],
  };
  const old = {
    conversations: {
      [id]: {
        title: "Old",
        parent: null,
        owner: null,
        events: [],
        inbox: [[1, JSON.stringify(message)]],
      },
    },
    receipts: {
      create: [
        JSON.stringify({ type: "create", title: "Old" }),
        { conversation: id },
      ],
    },
  };
  const data = legacyRoom(old);
  assert.equal(data.outbox[1].message_id, message.id);
  accept(data, { type: "create", request_id: "create", title: "Old" }, id);
  assert.equal(Object.keys(data.conversations).length, 1);
  assert.equal(data.outbox.length, 2);
});

test("untrusted dictionary keys cannot mutate prototypes or lose durable receipts", () => {
  let data = emptyRoom();
  const command = {
    type: "create",
    request_id: "__proto__",
    title: "Prototype-safe",
  };
  accept(data, command, id);
  data = JSON.parse(JSON.stringify(data));
  accept(data, command, id);
  assert.equal(data.outbox.length, 1);
  assert.equal(Object.hasOwn(data.receipts, "__proto__"), true);
  assert.throws(
    () =>
      accept(
        data,
        { type: "cancel_schedule", request_id: "bad", schedule: "__proto__" },
        id,
      ),
    /unknown schedule/,
  );
  assert.throws(
    () =>
      accept(
        data,
        {
          type: "send",
          request_id: "bad-send",
          conversation: "__proto__",
          text: "hello",
        },
        id,
      ),
    /unknown conversation/,
  );
  assert.equal(Object.prototype.active, undefined);
});

test("fork/import seeds preserve exclusion of search-generated evidence", () => {
  const message = {
    id: "message_summary",
    role: "agent",
    content: [{ type: "text", text: "Synthesized search answer" }],
  };
  const events = [
    {
      type: "tool.started",
      sequence: 1,
      context: { turn_id: "turn_search" },
      data: { tool_name: "search_history" },
    },
    {
      type: "output.message.completed",
      sequence: 2,
      context: { turn_id: "turn_search" },
      data: { message },
    },
  ];
  const [seed] = historySeed(events);
  assert.equal(seed.metadata.chat_history_evidence, false);
  assert.equal(
    message.metadata,
    undefined,
    "source history must not be mutated",
  );
  const [forked] = historySeed([
    { ...events[1], context: {}, data: { message: seed } },
  ]);
  assert.equal(forked.metadata.chat_history_evidence, false);
});
