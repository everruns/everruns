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
    const last = body.messages.at(-1),
      names = (body.tools || []).map((t) => t.function.name);
    const child = names.includes("search_conversations");
    if (child && slowSearch && last.role !== "tool") await sleep(3000);
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
    if (last.role === "tool")
      message = {
        role: "assistant",
        content: child
          ? "PostgreSQL; see the source conversation in the search result."
          : "Done: PostgreSQL.",
      };
    else if (child)
      message = tool("search_conversations", { query: "PostgreSQL" });
    else if (last.content.includes("What did"))
      message = tool("search_history", {
        question: "What did we decide about the event store?",
      });
    else if (last.content.includes("Schedule test"))
      message = tool("schedule", {
        instruction: "Scheduled follow-up has fired",
        delay_seconds: 4,
      });
    else
      message = { role: "assistant", content: "Remembered: " + last.content };
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ choices: [{ message }] }));
  });
});
await new Promise((r) => model.listen(0, "127.0.0.1", r));
// Construct explicitly: no credentials or mutable files from a developer's checkout.
const config = {
  name: "everruns-chat-test",
  main: join(root, "build/worker/shim.mjs"),
  compatibility_date: "2026-10-01",
  durable_objects: {
    bindings: [{ name: "ROOMS", class_name: "ChatRoom" }],
  },
  migrations: [{ tag: "v1", new_sqlite_classes: ["ChatRoom"] }],
  vars: {
    MODEL: "scripted",
    CHAT_ACCESS_TOKEN: "test-access",
    OPENROUTER_API_KEY: "test",
    OPENROUTER_BASE_URL: `http://127.0.0.1:${model.address().port}/v1`,
    LEASE_MS: "1000",
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
            authorization: "Bearer test-access",
          },
          body: JSON.stringify(body),
        }
      : { headers: { authorization: "Bearer test-access" } },
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
  return (
    c && !c.turn && !c.queued && c.messages.some((m) => m.role === "agent") && c
  );
};
async function socket() {
  const ws = new WebSocket(
    base.replace("http", "ws") + "/rooms/shared/socket?access=test-access",
  );
  const messages = [];
  ws.addEventListener("message", (e) => messages.push(JSON.parse(e.data)));
  await new Promise((r, j) => {
    ws.addEventListener("open", r, { once: true });
    ws.addEventListener("error", j, { once: true });
  });
  return { ws, messages };
}
try {
  processHandle = start();
  await until("Worker ready", () => state(), 90000);
  assert.equal((await fetch(base + "/rooms/shared/state")).status, 401);
  assert.equal((await fetch(base + "/cells/private/state")).status, 401);
  assert.equal(
    (
      await fetch(base + "/cells/private/state", {
        headers: { authorization: "Bearer test-access" },
      })
    ).status,
    404,
  );
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
  assert.equal(firstState.messages.filter((m) => m.role === "user").length, 1);
  const boundary = firstState.messages.find((m) => m.role === "agent").sequence;
  const fork = (
    await cmd({
      type: "fork",
      request_id: "fork",
      conversation: first,
      sequence: boundary,
    })
  ).conversation;
  assert.equal(conversation(await state(), fork).messages.length, 2);
  assert.equal((await state("isolated")).conversations.length, 0);
  console.log("chat e2e: sockets, receipts, forks and room isolation passed");
  // Restart while the search child is calling the model. No client or GET
  // request after restart may be needed to make the pending turn continue.
  const question = (await create("create-two", "Decisions")).conversation;
  slowSearch = true;
  await send(question, "What did we decide about the event store?", "search");
  await until("search model started", () =>
    calls.some((c) =>
      c.tools?.some((t) => t.function.name === "search_conversations"),
    ),
  );
  await kill();
  b.ws.close();
  slowSearch = false;
  const before = calls.length;
  processHandle = start();
  await until(
    "recovery alarm calls model without clients",
    () => calls.length > before,
    90000,
  );
  const answer = await until("search finishes", () => idle(question));
  assert.ok(answer.messages.at(-1).text.includes("PostgreSQL"));
  const afterSearch = await state();
  assert.equal(afterSearch.conversations.filter((c) => c.owner).length, 1);
  assert.equal(
    calls.filter(
      (c) =>
        c.tools?.some((t) => t.function.name === "search_history") &&
        c.messages.at(-1).content?.includes("Alice: What did"),
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
  await until("scheduled turn ends", async () => {
    const c = await idle(question);
    return c?.messages.some((m) => m.text?.includes("[Scheduled instruction]"));
  });
  const final = await state();
  assert.equal(
    conversation(final, question).messages.filter(
      (m) => m.role === "user" && m.text.includes("[Scheduled instruction]"),
    ).length,
    1,
  );
  console.log(
    "chat e2e: shared sockets, reconnect deduplication, forks, isolation, search recovery, and scheduled wake after restart passed",
  );
} catch (e) {
  console.error(logs.slice(-7000));

  throw e;
} finally {
  await kill();
  model.close();
  model.closeAllConnections();
  await rm(temp, { recursive: true, force: true });
}
