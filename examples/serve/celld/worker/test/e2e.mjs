// End to end on a real celld node: the cell restores a lost serve process and
// replays the turn the loss interrupted.
//
//   cargo build -p serve-example-celld
//   node test/e2e.mjs
//
// Uses the external cell (SERVE_URL) so it needs no container engine: killing
// the serve process and starting a new one with an empty data directory is
// what a container lost with its node looks like to the cell. Then the celld
// node itself is killed with a journaled turn outstanding, and the cell's
// alarm must snapshot it after the restart.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const worker = join(here, "..");
const repo = join(worker, "..", "..", "..", "..");
const binary = process.env.SERVE_BIN ?? join(repo, "target", "debug", "celld-agent");
const servePort = 18000 + Math.floor(Math.random() * 1000);
const cellPort = 19000 + Math.floor(Math.random() * 1000);
const base = `http://127.0.0.1:${cellPort}/cells/e2e`;
const children = new Set();

function start(cmd, args, opts) {
  const child = spawn(cmd, args, { stdio: ["ignore", "pipe", "pipe"], ...opts });
  child.log = "";
  child.stdout.on("data", (d) => (child.log += d));
  child.stderr.on("data", (d) => (child.log += d));
  children.add(child);
  return child;
}

function kill(child) {
  return new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) return resolve();
    child.once("exit", resolve);
    child.kill("SIGKILL");
  });
}

// One path, wiped per start: a new container has the same image layout and
// an empty disk. (The engine records the workspace root, so a restore must
// land at the path it was taken from.)
const dataDir = join(mkdtempSync(join(tmpdir(), "serve-celld-e2e-")), "data");

function startServe() {
  rmSync(dataDir, { recursive: true, force: true });
  return start(binary, ["celld", "--dev"], {
    env: {
      ...process.env,
      PORT: String(servePort),
      SERVE_DATA_DIR: dataDir,
      RESEARCH_DELAY_MS: "3000",
    },
  });
}

function startCelld() {
  return start("celld", ["dev", "wrangler.external.jsonc", "--port", String(cellPort), "--no-watch", "--logs"], {
    cwd: worker,
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function until(what, fn, ms = 30_000) {
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
  return text ? JSON.parse(text) : null;
}

const cell = () => json("/_cell/state");

async function say(session, text) {
  return json(`/v1/sessions/${session}/messages`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ message: { role: "user", content: [{ type: "text", text }] } }),
  });
}

// The texts of the session's user and agent messages, in order.
async function transcript(session) {
  const page = await json(`/v1/sessions/${session}/events?limit=1000`);
  const events = page.data ?? page.events ?? page;
  const out = [];
  for (const event of events) {
    const message = event.data?.message;
    if (!message || !["input.message", "output.message.completed"].includes(event.type)) continue;
    const text = (message.content ?? [])
      .filter((part) => part.type === "text")
      .map((part) => part.text)
      .join("");
    if (text) out.push(`${event.type === "input.message" ? "user" : "agent"}: ${text}`);
  }
  return out;
}

async function idle(session) {
  const s = await json(`/v1/sessions/${session}`);
  return s.status === "idle";
}

async function main() {
  writeFileSync(join(worker, ".dev.vars"), `SERVE_URL=http://127.0.0.1:${servePort}\n`);
  rmSync(join(worker, ".celld"), { recursive: true, force: true });
  let serve = startServe();
  let celld = startCelld();
  await until("serve", () => fetch(`http://127.0.0.1:${servePort}/celld/state`).then((r) => r.ok));
  await until("celld", () => fetch(base + "/_cell/state").then((r) => r.ok), 120_000);

  // A created session is acknowledged only once a snapshot holds it.
  const session = (
    await json("/v1/sessions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    })
  ).id;
  let state = await cell();
  assert.equal(state.snapshot_seq, 1, "create takes a snapshot");
  assert.equal(state.journal, 0);

  // A finished turn is folded into the next snapshot by the cell's alarm.
  await say(session, "first");
  await until("first turn snapshotted", async () => {
    const s = await cell();
    return s.journal === 0 && s.snapshot_seq >= 2 && (await idle(session));
  });

  // Lose the container mid-turn: the message is journaled, not snapshotted.
  await say(session, "second");
  await sleep(500);
  assert.equal((await json(`/v1/sessions/${session}`)).status, "active");
  await kill(serve);
  state = await cell();
  assert.equal(state.journal, 1, "the interrupted turn is journaled");

  // A fresh container with an empty disk: the next request restores and
  // replays, and the interrupted turn runs to the end exactly once.
  serve = startServe();
  await until("new serve", () => fetch(`http://127.0.0.1:${servePort}/celld/state`).then((r) => r.ok));
  await until("replayed turn finished", () => idle(session), 60_000);
  const lines = await transcript(session);
  console.log(lines.join("\n"));
  assert.deepEqual(
    lines.filter((line) => line.startsWith("user:")),
    ["user: first", "user: second"],
    "each message once, in order",
  );
  assert.equal(lines.filter((line) => line.startsWith("agent:")).length, 2, "both turns answered");
  state = await cell();
  assert.equal(state.restores, 1);
  assert.equal(state.replayed, 1);

  // Lose the celld node with a journaled turn outstanding. The alarm is
  // durable, so the restarted node snapshots the turn without any request.
  const before = (await until("second turn snapshotted", async () => {
    const s = await cell();
    return s.journal === 0 && s;
  })).snapshot_seq;
  await say(session, "third");
  await kill(celld);
  await sleep(4000); // the turn finishes in the container meanwhile
  celld = startCelld();
  await until(
    "alarm snapshot after node restart",
    async () => {
      const r = await fetch(base + "/_cell/state");
      if (!r.ok) return false;
      const s = await r.json();
      return s.journal === 0 && s.snapshot_seq > before && s;
    },
    120_000,
  );
  assert.equal((await transcript(session)).filter((l) => l.startsWith("agent:")).length, 3);

  // The container's control routes are not reachable from outside.
  assert.equal((await fetch(base + "/celld/snapshot")).status, 404);
  console.log("e2e: ok", await cell());
}

try {
  await main();
} catch (err) {
  for (const child of children) console.error(`--- ${child.spawnfile} log ---\n${child.log.slice(-4000)}`);
  throw err;
} finally {
  await Promise.all([...children].map(kill));
  rmSync(join(worker, ".dev.vars"), { force: true });
}
