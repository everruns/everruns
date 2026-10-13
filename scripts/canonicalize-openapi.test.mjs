import assert from "node:assert/strict";
import { test } from "node:test";
import { canonicalize } from "./canonicalize-openapi.mjs";

test("extension order is stable without reordering standard fields or arrays", () => {
  const a = { summary: "Run", tags: ["B", "A"], "x-z": { "x-b": 2, type: "object", "x-a": 1 }, "x-a": true };
  const b = { summary: "Run", "x-a": true, tags: ["B", "A"], "x-z": { type: "object", "x-a": 1, "x-b": 2 } };
  assert.equal(JSON.stringify(canonicalize(a)), JSON.stringify(canonicalize(b)));
  assert.deepEqual(Object.keys(canonicalize(a)), ["summary", "tags", "x-a", "x-z"]);
  assert.deepEqual(canonicalize(a).tags, ["B", "A"]);
  assert.deepEqual(Object.keys(canonicalize(a)["x-z"]), ["type", "x-a", "x-b"]);
  assert.deepEqual(canonicalize(canonicalize(a)), canonicalize(a));
});
