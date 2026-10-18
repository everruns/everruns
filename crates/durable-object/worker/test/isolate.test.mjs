import test from "node:test";
import assert from "node:assert/strict";
import { objectPort } from "../src/isolate.js";
import { fixture, input, tenant, sessionId } from "./support.mjs";

test("isolate bridge confines calls to the activation and preserves ownership refusals", async () => {
  const f = await fixture();
  const port = objectPort(f.authority, { tenant, session_id: sessionId });
  const call = async (operation, body = {}, scope = {}) =>
    JSON.parse(
      await port(JSON.stringify({ tenant, session: sessionId, operation, body, ...scope })),
    );
  assert.equal((await call("status", {}, { tenant: "other" })).status, 404);
  assert.equal((await call("status", {}, { session: "other" })).status, 404);
  assert.equal(JSON.parse(await port("x".repeat(1024 * 1024 + 1))).status, 413);
  assert.equal(JSON.parse(await port("invalid")).status, 503);
  assert.equal((await call("input", { event: input() })).status, 200);
  assert.equal(
    (await call("append", { lease: { holder: "old", fence: 0 }, event: input() })).status,
    412,
  );
});

test("isolate bridge never exposes unexpected authority errors", async () => {
  const port = objectPort(
    {
      handle() {
        throw new Error("private credential");
      },
    },
    { tenant, session_id: sessionId },
  );
  const result = await port(
    JSON.stringify({ tenant, session: sessionId, operation: "status", body: {} }),
  );
  assert.deepEqual(JSON.parse(result), { status: 503, value: null });
  assert.ok(!result.includes("private"));
});
