import test from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { z } from "zod";

const { version } = createRequire(import.meta.url)("zod/package.json");

test("the active v4 schema engine is at least the published security fix", () => {
  const [major, minor, patch] = version.split(".").map(Number);
  assert.ok(major > 4 || (major === 4 && (minor > 6 || (minor === 6 && patch >= 6))));
});

test("oversized constrained arrays reject before element validators run", () => {
  let visited = 0;
  const element = z.string().refine(() => { visited++; return true; });
  for (const schema of [z.array(element).max(2), z.array(element).length(2), z.object({ values: z.array(element).max(2) })]) {
    visited = 0;
    const input = schema instanceof z.ZodObject ? { values: ["a", "b", "c"] } : ["a", "b", "c"];
    const result = schema.safeParse(input);
    assert.equal(result.success, false);
    assert.equal(result.error.issues.length, 1);
    assert.equal(result.error.issues[0].code, "too_big");
    assert.equal(visited, 0);
  }
  visited = 0;
  assert.deepEqual(z.array(element).max(2).parse(["a", "b"]), ["a", "b"]);
  assert.equal(visited, 2);
});

test("oversized map and set inputs reject before walking their elements", () => {
  let visited = 0;
  const value = z.string().refine(() => { visited++; return true; });
  const map = z.map(value, value).max(1).safeParse(new Map([["a", "b"], ["c", "d"]]));
  assert.equal(map.success, false);
  assert.equal(map.error.issues.length, 1);
  assert.equal(visited, 0);
  const set = z.set(value).max(1).safeParse(new Set(["a", "b"]));
  assert.equal(set.success, false);
  assert.equal(set.error.issues.length, 1);
  assert.equal(visited, 0);
});
