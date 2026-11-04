import assert from "node:assert/strict";
import test from "node:test";
import { eventsForMessage } from "./compose-e2e-turn.mjs";

test("a prior completed turn cannot finish the newly submitted Chat request", () => {
  const previous = { type: "turn.completed", context: { turn_id: "old" } };
  const input = { type: "input.message", context: { input_message_id: "new-input" } };
  assert.deepEqual(eventsForMessage([previous, input], "new-input"), [input]);
  const started = {
    type: "turn.started",
    data: { input_message_id: "new-input", turn_id: "new-turn" },
    context: { turn_id: "new-turn" },
  };
  const failed = { type: "turn.failed", context: { turn_id: "new-turn" } };
  assert.deepEqual(eventsForMessage([previous, input, started, failed], "new-input"), [
    input,
    started,
    failed,
  ]);
});
