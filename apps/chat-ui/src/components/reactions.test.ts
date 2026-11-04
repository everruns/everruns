import { test } from "node:test";
import assert from "node:assert/strict";
import { applyReaction, reactionVersion, type ReactionsByMessage } from "./reactions";
import { conversationMessages, type ChatEvent } from "./transcript";

test("a reaction note is not a row in the conversation", () => {
  const events: ChatEvent[] = [
    {
      type: "output.message.completed",
      data: { message: { id: "message_a", role: "agent", content: [{ type: "text", text: "Done." }] } },
    },
    {
      type: "input.message",
      data: {
        message: {
          id: "message_n",
          role: "user",
          content: [{ type: "text", text: "[Reactions, not typed messages]\nJD reacted 🙏" }],
          metadata: { everruns_origin: { kind: "reaction" } },
        },
      },
    },
  ];
  assert.deepEqual(
    conversationMessages(events).map((message) => message.text),
    ["Done."],
  );
});

test("the newest reaction event is the version", () => {
  assert.equal(reactionVersion([{ type: "input.message", data: {} }]), null);
  assert.equal(
    reactionVersion([
      { id: "e1", type: "message.reaction.added", data: {} },
      { id: "e2", type: "output.message.completed", data: {} },
      { id: "e3", type: "message.reaction.removed", data: {} },
    ]),
    "e3",
  );
});

test("adding and removing the consumer's reaction updates counts", () => {
  const start: ReactionsByMessage = new Map([
    ["m", [{ emoji: "👍", count: 1, me: false, agent: true }]],
  ]);
  const added = applyReaction(start, "m", "👍", true);
  assert.deepEqual(added.get("m"), [{ emoji: "👍", count: 2, me: true, agent: true }]);
  assert.equal(applyReaction(added, "m", "👍", true), added);
  const removed = applyReaction(added, "m", "👍", false);
  assert.deepEqual(removed.get("m"), [{ emoji: "👍", count: 1, me: false, agent: true }]);
  const fresh = applyReaction(new Map(), "x", "🎉", true);
  assert.equal(applyReaction(fresh, "x", "🎉", false).has("x"), false);
});
