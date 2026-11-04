import test from "node:test";
import assert from "node:assert/strict";
import { fetchModel } from "../../../../examples/durable-object-agent/worker/provider.js";
const env = { OPENAI_BASE_URL: "https://provider.example/v1", OPENAI_API_KEY: "private-test-key" };
const request = JSON.stringify({
  method: "POST",
  path: "/chat/completions",
  headers: [
    ["authorization", "Bearer caller-key"],
    ["content-type", "application/json"],
  ],
  body: '{"stream":true}',
});
test("provider fetch uses configured endpoint/auth and refuses redirects while preserving shared protocol bytes", async () => {
  const signal = new AbortController().signal;
  const expected = new Response("SSE bytes", { headers: { "content-type": "text/event-stream" } });
  const response = await fetchModel(env, request, signal, async (url, options) => {
    assert.equal(String(url), "https://provider.example/v1/chat/completions");
    assert.equal(options.redirect, "manual");
    assert.equal(options.headers.get("authorization"), "Bearer private-test-key");
    assert.equal(options.body, '{"stream":true}');
    return expected;
  });
  assert.equal(response, expected);
});
test("provider cancellation reaches the in-flight host fetch", async () => {
  const controller = new AbortController();
  let observed;
  await fetchModel(env, request, controller.signal, async (_url, options) => {
    observed = options.signal;
    return new Response();
  });
  assert.equal(observed.aborted, false);
  controller.abort();
  assert.equal(observed.aborted, true);
});
test("invalid endpoint, missing credential and unsupported operation fail before host I/O", async () => {
  const fail = () => assert.fail("invalid requests must never reach fetch");
  for (const OPENAI_BASE_URL of [
    "http://public.example/v1",
    "https://user:password@provider.example/v1",
    "https://provider.example/v1?secret=x",
  ])
    await assert.rejects(
      fetchModel({ ...env, OPENAI_BASE_URL }, request, new AbortController().signal, fail),
    );
  await assert.rejects(
    fetchModel({ ...env, OPENAI_API_KEY: "" }, request, new AbortController().signal, fail),
  );
  await assert.rejects(
    fetchModel(
      env,
      JSON.stringify({ method: "DELETE", path: "/steal" }),
      new AbortController().signal,
      fail,
    ),
  );
});
