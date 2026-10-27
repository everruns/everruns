// Real workerd + SQLite + hibernatable sockets + alarms. The scripted model
// is confined to this test; the runnable app always calls a real provider.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const temp = await mkdtemp(join(tmpdir(), "everruns-chat-"));
const port = 24000 + Math.floor(Math.random() * 1000),
  base = `http://127.0.0.1:${port}`;
const calls = [];
let slowSearch = false,
  processHandle,
  logs = "";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const model = createServer((req, res) => {
  let raw = "";
  req.on("data", (d) => (raw += d));
  req.on("end", async () => {
    const body = JSON.parse(raw);
    calls.push(body);
    const inputs = body.input;
    const last = inputs.at(-1),
      names = (body.tools || []).map((t) => t.name);
    const lastText =
      typeof last.content === "string"
        ? last.content
        : (last.content || []).map((p) => p.text || "").join("");
    const isResult = last.type === "function_call_output";
    const child = names.includes("search_conversations");
    if (child && slowSearch && !isResult) await sleep(3000);
    let message;
    const tool = (name, args) => ({
      role: "assistant",
      content: null,
      tool_calls: [
        {
          id: `call_${calls.length}`,
          type: "function",
          function: { name, arguments: JSON.stringify(args) },
        },
      ],
    });
    if (isResult)
      message = {
        role: "assistant",
        content: child
          ? "PostgreSQL; see the source conversation in the search result."
          : "Done: PostgreSQL.",
      };
    else if (child)
      message = tool("search_conversations", { query: "PostgreSQL" });
    else if (lastText.includes("What did"))
      message = tool("search_history", {
        question: "What did we decide about the event store?",
      });
    else if (lastText.includes("Schedule test"))
      message = tool("schedule", {
        instruction: "Scheduled follow-up has fired",
        delay_seconds: 4,
      });
    else message = { role: "assistant", content: "Remembered: " + lastText };
    const output = message.tool_calls?.map((t) => ({
      type: "function_call",
      id: t.id,
      call_id: t.id,
      status: "completed",
      name: t.function.name,
      arguments: t.function.arguments,
    })) ?? [
      {
        type: "message",
        id: `msg_${calls.length}`,
        role: "assistant",
        status: "completed",
        content: [
          { type: "output_text", text: message.content, annotations: [] },
        ],
      },
    ];
    const response = {
      id: `resp_${calls.length}`,
      object: "response",
      status: "completed",
      model: body.model,
      output,
      usage: { input_tokens: 10, output_tokens: 10, total_tokens: 20 },
    };
    res.setHeader("content-type", "text/event-stream");
    const events = [];
    if (message.tool_calls) {
      for (const [index, item] of output.entries()) {
        events.push({
          type: "response.output_item.added",
          output_index: index,
          item: { ...item, status: "in_progress", arguments: "" },
        });
        events.push({
          type: "response.function_call_arguments.delta",
          output_index: index,
          item_id: item.id,
          delta: item.arguments,
        });
        events.push({
          type: "response.output_item.done",
          output_index: index,
          item,
        });
      }
    } else
      events.push({
        type: "response.output_text.delta",
        delta: message.content,
      });
    events.push({ type: "response.completed", response });
    res.end(
      events.map((event) => `data: ${JSON.stringify(event)}\n\n`).join(""),
    );
  });
});
await new Promise((r) => model.listen(0, "127.0.0.1", r));
// Construct explicitly: no credentials or mutable files from a developer's checkout.
const config = {
  name: "everruns-chat-test",
  main: join(root, "test/worker-fixture.js"),
  compatibility_flags: ["nodejs_compat"],
  rules: [{ type: "Text", globs: ["**/*.html"], fallthrough: true }],
  compatibility_date: "2026-10-01",
  durable_objects: {
    bindings: [
      { name: "ROOMS", class_name: "ChatRoom" },
      { name: "SESSIONS", class_name: "ChatSession" },
    ],
  },
  migrations: [{ tag: "v1", new_sqlite_classes: ["ChatRoom", "ChatSession"] }],
  vars: {
    MODEL: "scripted",
    LEGACY_MODEL: "scripted",
    TEST_CREATION_BUILD: "chat-71eae7eb9de56959711746bb",
    TENANT_ID: "chat",
    OPENROUTER_API_KEY: "test",
    OPENROUTER_BASE_URL: `http://127.0.0.1:${model.address().port}/v1`,
  },
};
await writeFile(join(temp, "wrangler.json"), JSON.stringify(config));
function start() {
  const child = spawn(
    "wrangler",
    [
      "dev",
      "--local",
      "--config",
      join(temp, "wrangler.json"),
      "--port",
      String(port),
      "--persist-to",
      join(temp, "state"),
      "--show-interactive-dev-session=false",
    ],
    {
      cwd: temp,
      detached: true,
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, WRANGLER_SEND_METRICS: "false" },
    },
  );
  child.stdout.on("data", (d) => (logs += d));
  child.stderr.on("data", (d) => (logs += d));
  return child;
}
async function kill() {
  if (
    !processHandle ||
    processHandle.exitCode !== null ||
    processHandle.signalCode !== null
  )
    return;
  const done = new Promise((r) => processHandle.once("exit", r));
  process.kill(-processHandle.pid, "SIGKILL");
  await done;
}
async function until(label, fn, ms = 45000) {
  const end = Date.now() + ms;
  let value;
  while (Date.now() < end) {
    try {
      value = await fn();
      if (value) return value;
    } catch (e) {
      value = e;
    }
    await sleep(100);
  }
  throw new Error(`timeout ${label}: ${value?.stack || JSON.stringify(value)}`);
}
async function request(path, body) {
  const r = await fetch(
    base + path,
    body
      ? {
          method: "POST",
          headers: {
            "content-type": "application/json",
          },
          body: JSON.stringify(body),
        }
      : undefined,
  );
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}
const state = (room = "shared") => request(`/rooms/${room}/state`);
const cmd = (body, room = "shared") => request(`/rooms/${room}/commands`, body);
const create = (id, title) => cmd({ type: "create", request_id: id, title });
const send = (id, text, key) =>
  cmd({
    type: "send",
    request_id: key,
    conversation: id,
    username: "Alice",
    text,
  });
const conversation = (s, id) => s.conversations.find((c) => c.id === id);
const idle = async (id) => {
  const c = conversation(await state(), id);
  if (c?.activity.some((e) => e.type === "turn.failed"))
    throw new Error(
      "Framework turn failed: " +
        JSON.stringify(c.activity.find((e) => e.type === "turn.failed").data),
    );
  return (
    c && !c.turn && !c.queued && c.messages.some((m) => m.role === "agent") && c
  );
};
async function socket() {
  const ws = new WebSocket(base.replace("http", "ws") + "/rooms/shared/socket");
  const messages = [];
  ws.addEventListener("message", (e) => {
    const message = JSON.parse(e.data);
    messages.push(message);
    if (message.type === "changed")
      state()
        .then((snapshot) => messages.push(snapshot))
        .catch(() => {});
  });
  await new Promise((r, j) => {
    ws.addEventListener("open", r, { once: true });
    ws.addEventListener("error", j, { once: true });
  });
  return { ws, messages };
}
try {
  processHandle = start();
  await until("Worker ready", () => state(), 90000);
  // Import actual old-format SQLite data; unfinished legacy work is not replayed.
  const legacyId = "session_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const oldMessage = (id, role, text) => ({
    id,
    role,
    created_at: "2026-10-16T00:00:00Z",
    content: [{ type: "text", text }],
  });
  const prior = oldMessage(
    "message_11111111111111111111111111111111",
    "user",
    "Legacy fact: PostgreSQL",
  );
  const reply = oldMessage(
    "message_22222222222222222222222222222222",
    "agent",
    "Remembered legacy fact.",
  );
  const pending = oldMessage(
    "message_33333333333333333333333333333333",
    "user",
    "A queued legacy follow-up",
  );
  const oldEvents = [
    [
      1,
      JSON.stringify({
        type: "input.message",
        sequence: 1,
        data: { message: prior },
      }),
      true,
    ],
    [
      2,
      JSON.stringify({
        type: "output.message.completed",
        sequence: 2,
        data: { message: reply },
      }),
      true,
    ],
    [
      3,
      JSON.stringify({
        type: "output.message.completed",
        sequence: 3,
        data: {
          message: oldMessage(
            "message_44444444444444444444444444444444",
            "agent",
            "Uncommitted legacy text",
          ),
        },
      }),
      false,
    ],
  ];
  await cmd(
    {
      type: "__seed_legacy",
      data: {
        revision: 1,
        conversations: {
          [legacyId]: {
            title: "Legacy",
            parent: null,
            owner: null,
            events: oldEvents,
            inbox: [[1, JSON.stringify(pending)]],
          },
        },
        schedules: {},
        receipts: {},
      },
    },
    "legacy",
  );
  const migrated = await until(
    "legacy history and queued input imported",
    async () => {
      const c = conversation(await state("legacy"), legacyId);
      return c?.ready && !c.turn && !c.queued && c.messages.length === 4 && c;
    },
  );
  assert.deepEqual(
    migrated.messages.filter((m) => m.role === "user").map((m) => m.id),
    [prior.id, pending.id],
  );
  assert.equal(migrated.messages[1].id, reply.id);
  const backup = await cmd({ type: "__legacy_backup" }, "legacy");
  assert.deepEqual(backup.conversations[legacyId].events, oldEvents);
  assert.equal(
    calls.filter((c) => c.metadata.session_id === legacyId).length,
    1,
    "uncertain admission acknowledgement must not repeat the model turn",
  );
  console.log(
    "chat e2e: legacy SQLite import, backup, and uncertain admission acknowledgement passed",
  );
  const page = await fetch(base + "/");
  assert.equal(page.status, 200);
  assert.equal(page.headers.get("set-cookie"), null);
  assert.match(await page.text(), /Message the room/);
  assert.equal((await fetch(base + "/rooms/shared/state")).status, 200);
  // These must stop at the production gateway before reaching either binding.
  // Forged authority headers and payloads cannot turn a public chat request into
  // an internal tool/authority operation. No service secret is configured.
  const privatePaths = [
    "/cells/private/state",
    "/internal/shared/tool",
    "/internal/shared/changed",
    "/%69nternal/shared/tool",
    "/rooms/shared/commands/internal/shared/tool",
    "/tenants/chat/sessions/session_" + "1".repeat(32) + "/engine",
    "/tenants/chat/sessions/session_" + "1".repeat(32) + "/watch",
    "/rooms/shared/%2e%2e/%2e%2e/internal/shared/tool",
  ];
  for (const path of privatePaths) {
    for (const method of ["GET", "POST"]) {
      const denied = await fetch(base + path, {
        method,
        headers: {
          authorization: "Bearer forged",
          "content-type": "application/json",
        },
        ...(method === "POST"
          ? { body: JSON.stringify({ operation: "create", type: "tool" }) }
          : {}),
      });
      assert.equal(denied.status, 404, `${method} ${path}`);
    }
  }

  const a = await socket(),
    b = await socket();
  a.ws.send(JSON.stringify({ type: "select", username: "Alice" }));
  b.ws.send(JSON.stringify({ type: "select", username: "Bob" }));
  const first = (await create("create-one", "Event store")).conversation;
  assert.equal((await create("create-one", "Event store")).conversation, first);
  a.ws.send(
    JSON.stringify({
      type: "send",
      request_id: "fact",
      conversation: first,
      username: "Alice",
      text: "We chose PostgreSQL for the event store.",
    }),
  );
  await until("ack", () =>
    a.messages.find((m) => m.type === "ack" && m.request_id === "fact"),
  );
  await until("both clients receive committed reply", () =>
    b.messages.find(
      (m) =>
        m.type === "snapshot" &&
        conversation(m, first)?.messages.some((x) => x.role === "agent"),
    ),
  );
  a.ws.close();
  await send(first, "We chose PostgreSQL for the event store.", "fact"); // retry same command after disconnect
  const firstState = await until("first turn", () => idle(first));
  assert.equal(firstState.execution.compatibility.status, "available");
  assert.equal(firstState.execution.build_id, config.vars.TEST_CREATION_BUILD);
  assert.equal(firstState.messages.filter((m) => m.role === "user").length, 1);
  assert.equal(calls.filter((c) => c.metadata.session_id === first).length, 1);
  const completion = firstState.activity.find(
    (e) => e.type === "turn.completed",
  ).data;
  assert.equal(typeof completion.duration_ms, "number");
  assert.equal(completion.usage.input_tokens, 10);
  assert.equal(completion.stop_reason, "stop");
  const boundary = firstState.messages.find((m) => m.role === "agent").sequence;
  const fork = (
    await cmd({
      type: "fork",
      request_id: "fork",
      conversation: first,
      sequence: boundary,
    })
  ).conversation;
  const forkState = await until("fork imported", async () => {
    const c = conversation(await state(), fork);
    return c?.ready && c;
  });
  assert.equal(forkState.messages.length, 2);
  await send(fork, "Continue this fork.", "fork-follow-up");
  await until("fork continues through Engine", () => idle(fork));
  const forkInput = calls.find((c) => c.metadata.session_id === fork).input;
  assert.ok(JSON.stringify(forkInput).includes("We chose PostgreSQL"));
  assert.ok(JSON.stringify(forkInput).includes("Continue this fork."));
  assert.equal((await idle(first)).messages.length, 2);
  assert.equal((await state("isolated")).conversations.length, 0);
  console.log("chat e2e: sockets, receipts, forks and room isolation passed");
  // Restart while the search child is calling the model. No client or GET
  // request after restart may be needed to make the pending turn continue.
  const question = (await create("create-two", "Decisions")).conversation;
  slowSearch = true;
  await send(question, "What did we decide about the event store?", "search");
  await until("search model started", () =>
    calls.some((c) => c.tools?.some((t) => t.name === "search_conversations")),
  );
  await kill();
  b.ws.close();
  slowSearch = false;
  const before = calls.length;
  // Host configuration may change on restart. Existing sessions must reconstruct
  // their retained model; only newly created sessions adopt the new default.
  config.vars.MODEL = "changed-host-model";
  config.vars.TEST_LEGACY_MEMBERSHIP_SESSION = first;
  delete config.vars.TEST_CREATION_BUILD;
  await writeFile(join(temp, "wrangler.json"), JSON.stringify(config));
  processHandle = start();
  await until(
    "recovery alarm calls model without clients",
    () => calls.length > before,
    90000,
  );
  await until(
    "parent resumes after child without clients",
    () =>
      calls.some(
        (c) =>
          c.tools?.some((t) => t.name === "search_history") &&
          c.input.at(-1)?.type === "function_call_output",
      ),
    90000,
  );
  await sleep(2000);
  const answer = conversation(await state(), question);
  assert.equal(
    answer.turn,
    null,
    "first read after quiet recovery must find the parent completed",
  );
  assert.equal(answer.queued, 0);
  assert.ok(answer.messages.at(-1).text.includes("PostgreSQL"));
  assert.equal(answer.execution.build_id, "chat-71eae7eb9de56959711746bb");
  assert.equal(answer.execution.compatibility.status, "available");
  assert.ok(
    calls.slice(before).every((c) => c.model === "scripted"),
    "recovered sessions must keep their original model after host config changes",
  );
  const afterSearch = await state();
  assert.equal(afterSearch.conversations.filter((c) => c.owner).length, 1);
  assert.equal(
    calls.filter(
      (c) =>
        c.tools?.some((t) => t.name === "search_history") &&
        JSON.stringify(c.input.at(-1)).includes("Alice: What did"),
    ).length,
    1,
    "committed parent Reason does not repeat",
  );
  console.log("chat e2e: interrupted child recovered without reconnecting");
  // Schedule a future instruction, then stop the entire runtime before due.
  await send(question, "Schedule test", "schedule");
  await until("schedule committed", async () =>
    Object.values((await state()).schedules).some((s) => s.active),
  );
  await until("scheduling turn ends", () => idle(question));
  await kill();
  const scheduledBefore = calls.length;
  processHandle = start();
  await until(
    "timer fires with no clients",
    () => calls.length > scheduledBefore,
    90000,
  );
  await sleep(2000);
  const final = await state();
  const scheduled = conversation(final, question);
  assert.equal(
    scheduled.turn,
    null,
    "first read after quiet timer recovery must find a completed turn",
  );
  assert.equal(scheduled.queued, 0);
  assert.equal(
    conversation(final, question).messages.filter(
      (m) => m.role === "user" && m.text.includes("[Scheduled instruction]"),
    ).length,
    1,
  );
  const fresh = (await create("create-new-model", "New default")).conversation;
  await send(fresh, "Use the new default", "new-model");
  await until("new model turn", () => idle(fresh));
  assert.notEqual(
    (await idle(fresh)).execution.build_id,
    "chat-71eae7eb9de56959711746bb",
    "new sessions use the current Agent registration while old bindings stay intact",
  );
  assert.equal(
    calls.find((c) => c.metadata.session_id === fresh).model,
    "changed-host-model",
    "new sessions adopt the configured default",
  );
  await send(
    first,
    "Continue after the legacy upgrade.",
    "legacy-model-upgrade",
  );
  await until("legacy membership continues", () => idle(first));
  assert.equal(
    calls.filter((c) => c.metadata.session_id === first).at(-1).model,
    "scripted",
    "pre-pinning memberships must use the historical model, not the new default",
  );
  console.log(
    "chat e2e: shared sockets, reconnect deduplication, forks, isolation, search recovery, and scheduled wake after restart passed",
  );
} catch (e) {
  await writeFile(
    "/tmp/everruns-chat-failed-calls.json",
    JSON.stringify(calls, null, 2),
  );
  await writeFile(
    "/tmp/everruns-chat-failed-state.json",
    JSON.stringify(await state().catch(() => null), null, 2),
  );
  console.error(logs.slice(-7000));

  throw e;
} finally {
  await kill();
  model.close();
  model.closeAllConnections();
  await rm(temp, { recursive: true, force: true });
}
