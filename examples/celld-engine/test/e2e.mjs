// End to end on a real celld node: the engine runs inside the cell, the node
// is killed in the middle of a tool call, and the cell's alarm finishes the
// turn on the restarted node without repeating the committed model call.
//
//   worker-build --release
//   node test/e2e.mjs
//
// The model is a scripted Chat Completions server in this process, so the
// test needs no key and can count the model calls the cell makes.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { writeFileSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const worker = join(dirname(fileURLToPath(import.meta.url)), "..");
const cellPort = 20000 + Math.floor(Math.random() * 1000);
const base = `http://127.0.0.1:${cellPort}/cells/e2e`;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let celld;

// The scripted model: ask for look_up, then answer once the result is in.
const calls = [];
const model = createServer((req, res) => {
  let body = "";
  req.on("data", (d) => (body += d));
  req.on("end", async () => {
    const request = JSON.parse(body);
    calls.push(request);
    await sleep(200);
    const last = request.messages.at(-1);
    const message =
      last.role === "tool"
        ? { role: "assistant", content: `celld runs Durable Objects. (answer ${calls.length})` }
        : {
            role: "assistant",
            content: null,
            tool_calls: [
              {
                id: `call_${calls.length}`,
                type: "function",
                function: { name: "look_up", arguments: JSON.stringify({ topic: last.content }) },
              },
            ],
          };
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ choices: [{ message }], usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 } }));
  });
});
await new Promise((r) => model.listen(0, "127.0.0.1", r));

function startCelld() {
  const child = spawn("celld", ["dev", "--port", String(cellPort), "--no-watch", "--logs"], {
    cwd: worker,
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.log = "";
  child.stdout.on("data", (d) => (child.log += d));
  child.stderr.on("data", (d) => (child.log += d));
  return child;
}

function kill(child) {
  return new Promise((resolve) => {
    if (!child || child.exitCode !== null || child.signalCode !== null) return resolve();
    child.once("exit", resolve);
    child.kill("SIGKILL");
  });
}

async function until(what, fn, ms = 60_000) {
  const deadline = Date.now() + ms;
  let last;
  while (Date.now() < deadline) {
    try {
      last = await fn();
      if (last) return last;
    } catch (err) {
      last = err;
    }
    await sleep(200);
  }
  throw new Error(`timed out waiting for ${what}: ${last?.stack ?? JSON.stringify(last)}`);
}

async function json(path, init) {
  const res = await fetch(base + path, init);
  const text = await res.text();
  if (!res.ok) throw new Error(`${init?.method ?? "GET"} ${path}: ${res.status} ${text}`);
  return JSON.parse(text);
}

const state = () => json("/state");
const say = (text) =>
  json("/messages", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ text }) });
const types = async () => (await json("/events")).data.map((e) => e.type);
const count = (list, type) => list.filter((t) => t === type).length;

async function main() {
  writeFileSync(
    join(worker, ".dev.vars"),
    [
      `OPENAI_BASE_URL=http://127.0.0.1:${model.address().port}/v1`,
      "OPENAI_API_KEY=test-key",
      "MODEL=scripted",
      "TOOL_DELAY_MS=4000",
      "LEASE_MS=3000",
    ].join("\n") + "\n",
  );
  rmSync(join(worker, ".celld"), { recursive: true, force: true });
  celld = startCelld();
  await until("celld", () => state(), 120_000);

  // A whole turn inside the cell: model, tool, model.
  await say("first");
  await until("first turn", async () => count(await types(), "turn.completed") === 1);
  assert.equal(calls.length, 2);
  assert.equal(calls[0].model, "scripted");
  assert.equal(calls[0].tools[0].function.name, "look_up");

  // Lose the node while the second turn's tool call runs.
  await say("second");
  await until("second turn's model call", () => calls.length === 3);
  await until("act step running", async () => (await state()).turn?.next?.step === "act");
  await sleep(500);
  await kill(celld);
  assert.equal(calls.length, 3);

  // Restart and send nothing: the durable alarm alone finishes the turn.
  celld = startCelld();
  await until("model call from the resumed turn", () => calls.length === 4, 120_000);
  const s = await until("second turn", async () => {
    const s = await state();
    return s.turn === null && s.inbox === 0 && s;
  });
  const log = await types();
  console.log(log.join("\n"));
  assert.equal(count(log, "input.message"), 2);
  assert.equal(count(log, "turn.started"), 2);
  assert.equal(count(log, "turn.completed"), 2);
  assert.equal(count(log, "tool.completed"), 2, "the lost tool call committed exactly once");
  assert.equal(count(log, "output.message.completed"), 4);
  assert.equal(calls.length, 4, "the committed model call was not repeated");
  assert.equal(s.resumed_steps, 1);
  assert.equal(calls[3].messages.filter((m) => m.role === "tool").length, 2);
  console.log("e2e: ok", s, { model_calls: calls.length });
}

try {
  await main();
} catch (err) {
  console.error(`--- celld log ---\n${celld?.log.slice(-6000)}`);
  throw err;
} finally {
  await kill(celld);
  model.close();
  rmSync(join(worker, ".dev.vars"), { force: true });
}
