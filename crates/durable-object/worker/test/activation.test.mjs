import test from "node:test";
import assert from "node:assert/strict";
import { runAlarm } from "../src/activation.js";
import { fixture, input, claim } from "./support.mjs";

const env = {
  EXECUTOR_URL: "https://executor.example/execute",
  EXECUTOR_TOKEN: "test-executor-secret-with-32-bytes",
};

test("an alarm retains its wake across downstream failure beyond platform retries", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  for (let attempt = 0; attempt < 10; attempt++) {
    f.storage.alarm = null; // The handler consumed the current alarm.
    await runAlarm(f.authority, env, async (_url, options) => {
      assert.equal(options.redirect, "manual");
      assert.ok(
        f.storage.alarm !== null,
        "next wake is durable before dispatch",
      );
      return new Response(null, { status: 503 });
    });
    assert.ok(f.storage.alarm !== null);
    f.advance(60001);
  }
  assert.equal((await f.call("status")).failures, 10);
  assert.equal((await f.call("status")).suspended, false);
});

test("duplicate alarm delivery honors stored backoff and active owner", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  let calls = 0;
  const downstream = async () => {
    calls++;
    return new Response(null, { status: 503 });
  };
  await runAlarm(f.authority, env, downstream);
  await runAlarm(f.authority, env, downstream);
  assert.equal(calls, 1);
  await claim(f);
  f.advance(2001);
  await runAlarm(f.authority, env, downstream);
  assert.equal(calls, 1);
});

test("incompatible executable identity suspends until an explicit repaired retry", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  await runAlarm(
    f.authority,
    env,
    async () => new Response(null, { status: 409 }),
  );
  assert.equal((await f.call("status")).suspended, true);
  await f.call("resume");
  assert.equal((await f.call("status")).suspended, false);
  assert.equal((await f.call("status")).retry_at, 0);
});

test("redirects never forward the executor secret and a stale failure cannot suspend a new owner", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  await runAlarm(
    f.authority,
    env,
    async () =>
      new Response(null, {
        status: 302,
        headers: { location: "https://another.example" },
      }),
  );
  assert.equal(
    (await f.call("status")).last_error,
    "invalid_executor_configuration",
  );
  f.advance(60001);
  await runAlarm(f.authority, env, async () => {
    await claim(f, "new-owner");
    return new Response(null, { status: 409 });
  });
  assert.equal((await f.call("status")).suspended, false);
});
