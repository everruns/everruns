// Run against workerd, an actual Cloudflare test deployment, or a celld fleet.
import assert from "node:assert/strict";
import { randomUUID, createHash } from "node:crypto";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
const base = process.env.DO_QUALIFY_URL;
const token = process.env.DO_QUALIFY_TOKEN ?? "authority-test-token-at-least-32-bytes";
const tenant = process.env.DO_QUALIFY_TENANT ?? "test";
const binding = JSON.parse(
  readFileSync(
    new URL("../../../../examples/durable-object-agent/worker/build.json", import.meta.url),
    "utf8",
  ),
);
const session = `session_${randomUUID().replaceAll("-", "")}`;
const apiToken = createHash("sha256").update(`${token}:api`).digest("hex");
async function api(path,method="GET",body) {
  const response = await fetch(`${base}/v1${path}`, {method,headers:{authorization:`Bearer ${apiToken}`,"content-type":"application/json"},...(body===undefined?{}:{body:JSON.stringify(body)}),signal:AbortSignal.timeout(15000)});
  const value = await response.json();
  assert.ok(response.ok,`${method} ${path}: ${response.status} ${JSON.stringify(value)}`);
  return value;
}
async function call(op, body = {}, options = {}) {
  const response = await fetch(
    `${base}/tenants/${options.tenant ?? tenant}/sessions/${session}/${op}`,
    {
      method: "POST",
      headers: {
        authorization: `Bearer ${options.token ?? token}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(15000),
    },
  );
  return { response, value: await response.json().catch(() => null) };
}
async function good(op, body) {
  const { response, value } = await call(op, body);
  assert.equal(response.status, 200, `${op}: ${JSON.stringify(value)}`);
  return value;
}
async function until(label, fn, ms = 90000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const result = await fn();
    if (result) return result;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`timed out: ${label}`);
}
assert.ok(base, "DO_QUALIFY_URL is required");
await until("authenticated endpoint ready", async () => {
  const { response, value } = await call("status", {}, { token: "wrong" });
  if (response.status === 401) return true;
  if (response.status !== 404 && response.status !== 503)
    throw new Error(`unexpected startup response: ${response.status} ${JSON.stringify(value)}`);
  return false;
});
assert.equal((await call("status", {}, { tenant: "other" })).response.status, 404);
await api("/sessions","POST",{id:session});
assert.equal(
  (await call("bind", { binding: { ...binding, build_id: "wrong-build" } })).response.status,
  409,
);
const input = {
  type: "input.message",
  ts: new Date().toISOString(),
  session_id: session,
  context: {},
  data: {
    message: {
      id: `message_${randomUUID().replaceAll("-", "")}`,
      role: "user",
      content: [{ type: "text", text: "Increment the counter once." }],
      created_at: new Date().toISOString(),
    },
  },
};
const send = {message_id:input.data.message.id,message:{role:"user",content:input.data.message.content}};
const accepted = await api(`/sessions/${session}/messages`,"POST",send);
assert.deepEqual(await api(`/sessions/${session}/messages`,"POST",send), accepted);
await until(
  "committed tool effect",
  async () =>
    (await good("store", {
      kind: "value",
      operation: "get",
      key: "counter",
    })) === "1",
);
const owner = await good("status");
assert.ok(owner.holder, "the tool's owner is still running");
if (process.env.DO_QUALIFY_SIGNAL_PATH) {
  const path = process.env.DO_QUALIFY_SIGNAL_PATH;
  writeFileSync(path, JSON.stringify({ session, owner }));
  await until("owner node killed", async () => existsSync(`${path}.continue`));
} else {
  try {
    await call("evict");
  } catch {} // abort intentionally closes the request.
}
// No application requests during recovery: a read must not supply activation.
await new Promise((r) => setTimeout(r, Number(process.env.DO_QUALIFY_NO_CLIENT_MS ?? 20000)));
const { data: events } = await api(`/sessions/${session}/events?limit=256`);
const outcome = await api(`/sessions/${session}/admissions/${input.data.message.id}`);
assert.equal(outcome.status,"complete","Framework finalization must survive owner loss");
assert.equal(outcome.turn.success,true);
assert.equal(
  await good("event_sequence", { event_id: events[0].id }),
  events[0].sequence,
  "opaque event cursors resolve through the authenticated authority route",
);
assert.ok(
  events.some((e) => e.type === "turn.completed"),
  "the stored alarm completed recovery without client requests",
);
assert.equal(await good("store", { kind: "value", operation: "get", key: "counter" }), "1");
for (const type of [
  "input.message",
  "turn.started",
  "turn.completed",
  "tool.started",
  "tool.completed",
])
  assert.equal(events.filter((e) => e.type === type).length, 1, type);
assert.equal(events.find((e) => e.type === "tool.completed").data.status, "interrupted");
const stale = await call("append", {
  lease: { holder: owner.holder, fence: owner.fence },
  event: events[0],
});
assert.equal(stale.response.status, 412);
console.log(
  JSON.stringify({
    backend: process.env.DO_QUALIFY_BACKEND ?? "workerd",
    placement: "wasm",
    entry: "everruns::Engine / Session",
    framework_finalized: true,
    admissions: 1,
    turns: 1,
    effects: 1,
    tool_status: "interrupted",
    activation: "stored alarm without client requests",
    stale_writer: "rejected",
  }),
);
