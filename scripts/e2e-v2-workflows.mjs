#!/usr/bin/env node
// Full-org workflow checks on data plane v2 (knowledge/execution/data-plane-v2/e2e-workflows.md).
//
// Starts the Compose stack (control plane plus agentd, v2 by default) from
// the images of one commit, runs each workflow a real org uses against it
// with a real model, and reports one pass or fail per workflow.
//
// Decisions:
// - A real model (GPT-6 Luna through OpenRouter): the workflows depend on
//   the model choosing tools, which a simulated model would only pretend.
// - Checks read events and records, never the reply's wording. A row that
//   fails gets one retry; both attempts are reported.
// - No skipped rows: a row that cannot run reports a failure.
// - The provider key reaches the server only through startup seeding, as
//   in scripts/e2e-compose-openrouter.mjs: it is never sent to the API,
//   printed or written outside the run's private .env.
// - The last check reads the database: the org must be placed on the data
//   plane and the server's own sessions table must stay empty, so every
//   row ran on v2.
import assert from "node:assert/strict";
import { randomBytes } from "node:crypto";
import { spawn } from "node:child_process";
import { appendFile, copyFile, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
if (process.argv.includes("--help")) {
  console.log(`Usage: OPENROUTER_API_KEY=... EVERRUNS_TAG=<image tag> node scripts/e2e-v2-workflows.mjs [row ...]
Rows: ${"chat coding threads"}. Default: every row.
Optional: OPENROUTER_MODEL (default openai/gpt-6-luna), E2E_PORT (default 19400),
E2E_REPORT (JSON report path), KEEP_STACK=1 (leave the stack running).
Writes a pass/fail table to $GITHUB_STEP_SUMMARY when set.`);
  process.exit(0);
}
const key = process.env.OPENROUTER_API_KEY;
assert(key, "OPENROUTER_API_KEY is required; there is no simulated fallback");
const tag = process.env.EVERRUNS_TAG || "latest";
const modelSlug = process.env.OPENROUTER_MODEL || "openai/gpt-6-luna";
const port = Number(process.env.E2E_PORT || 19400);
const base = `http://localhost:${port}`;
const apiBase = `${base}/api/v1/platform`;
const redact = (text) => String(text).replaceAll(key, "[REDACTED]");
const log = (text) => console.log(redact(text));
const delay = (ms) => new Promise((done) => setTimeout(done, ms));

// --- Stack ------------------------------------------------------------------

const work = await mkdtemp(join(tmpdir(), "everruns-e2e-v2-"));
const project = `everruns-e2e-v2-${randomBytes(4).toString("hex")}`;
const env = {
  ...Object.fromEntries(
    Object.entries(process.env).filter(([name]) =>
      ["PATH", "HOME", "DOCKER_HOST", "DOCKER_CONFIG", "TMPDIR"].includes(name),
    ),
  ),
  COMPOSE_PROJECT_NAME: project,
  EVERRUNS_TAG: tag,
  EXAMPLE_PROXY_BIND: "127.0.0.1",
  EXAMPLE_PROXY_PORT: String(port),
};

function compose(args, { timeout = 300_000, quiet = false } = {}) {
  if (!quiet) log(`$ docker compose ${args.join(" ")}`);
  return new Promise((done, fail) => {
    const child = spawn("docker", ["compose", ...args], { cwd: work, env });
    let out = "";
    child.stdout.on("data", (chunk) => (out += chunk));
    child.stderr.on("data", (chunk) => (out += chunk));
    const timer = setTimeout(() => child.kill("SIGTERM"), timeout);
    child.on("error", fail);
    child.on("close", (code) => {
      clearTimeout(timer);
      code === 0
        ? done(out)
        : fail(new Error(`docker compose ${args[0]} exited ${code}: ${redact(out.slice(-2000))}`));
    });
  });
}

async function startStack() {
  await copyFile(join(root, "infra/docker-compose/docker-compose.yaml"), join(work, "docker-compose.yaml"));
  const password = randomBytes(16).toString("hex");
  const lines = [
    `SECRETS_ENCRYPTION_KEY=kek-v1:${randomBytes(32).toString("base64")}`,
    `AUTH_JWT_SECRET=${randomBytes(32).toString("hex")}`,
    `WORKER_GRPC_AUTH_TOKEN=${randomBytes(32).toString("hex")}`,
    `EVERRUNS_DP_TOKEN=${randomBytes(32).toString("hex")}`,
    `EVERRUNS_DP_KEY=${randomBytes(32).toString("base64")}`,
    "AUTH_ADMIN_EMAIL=admin@example.com",
    `AUTH_ADMIN_PASSWORD=${password}`,
    `DEFAULT_OPENROUTER_API_KEY=${key}`,
    "SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV=true",
    "",
  ];
  await writeFile(join(work, ".env"), lines.join("\n"), { mode: 0o600 });
  await compose(["pull", "--quiet"], { timeout: 900_000 });
  await compose(["up", "-d", "--wait", "--wait-timeout", "240"], { timeout: 300_000 });
  for (let i = 0; i < 90; i++) {
    if ((await fetch(`${base}/health`).catch(() => undefined))?.ok) break;
    await delay(2000);
  }
  const health = await fetch(`${base}/health`);
  assert(health.ok, `GET /health: HTTP ${health.status}`);
  return password;
}

async function stopStack() {
  if (process.env.KEEP_STACK === "1") {
    log(`Stack left running: project ${project} in ${work}`);
    return;
  }
  await compose(["logs", "--no-color", "--tail", "200", "server", "agentd", "executor", "edge"], { quiet: true })
    .then((out) => writeFile(join(process.env.E2E_LOGS || work, `${project}.log`), redact(out)))
    .catch(() => {});
  await compose(["down", "-v", "--remove-orphans"], { timeout: 180_000 }).catch((e) => log(e.message));
  await rm(work, { recursive: true, force: true });
}

// --- API --------------------------------------------------------------------

let token;
async function api(path, method = "GET", body, { ok = true } = {}) {
  const response = await fetch(`${apiBase}${path}`, {
    method,
    headers: {
      "Content-Type": "application/json",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(300_000),
  });
  if (ok && !response.ok) {
    throw new Error(`${method} ${path}: HTTP ${response.status} ${(await response.text()).slice(0, 300)}`);
  }
  const text = await response.text();
  return { status: response.status, body: text ? JSON.parse(text) : undefined };
}
const get = async (path) => (await api(path)).body;
const post = async (path, body = {}) => (await api(path, "POST", body)).body;

async function signIn(password) {
  const { access_token } = await post("/auth/login", {
    email: "admin@example.com",
    password,
  });
  token = access_token;
  token = (await post("/auth/personal-access-tokens", { name: "e2e-v2" })).token;
  assert(token?.startsWith("evr_pat_"), "no personal access token");
}

/** The org's OpenRouter provider (seeded at startup) and the run's model. */
async function chooseModel() {
  const provider = (await get("/providers")).data.find((p) => p.provider_type === "openrouter");
  assert(provider?.api_key_set, "the OpenRouter key was not seeded into the default org");
  // 409: the startup seeding already synced this provider's catalog (seen on
  // the first CI run); the model lookup below covers either way.
  const sync = await api(`/providers/${provider.id}/sync-models`, "POST", {}, { ok: false });
  assert(sync.status < 300 || sync.status === 409, `sync-models: HTTP ${sync.status}`);
  let model = (await get("/models?limit=1000")).data.find(
    (m) => m.provider_id === provider.id && m.model_id === modelSlug,
  );
  model ??= await post(`/providers/${provider.id}/models`, {
    model_id: modelSlug,
    display_name: "E2E model",
    capabilities: ["chat", "tools"],
    enabled: true,
  });
  await api(`/models/${model.id}`, "PATCH", { enabled: true });
  return model.id;
}

let modelId;
const runId = randomBytes(3).toString("hex");
// Names are unique per org: a retried row creates its agents again, so each
// one gets its own counter.
let agents = 0;
async function agent(name, systemPrompt, capabilities = []) {
  const created = await post("/agents", {
    name: `e2e-${name}-${runId}-${++agents}`,
    system_prompt: systemPrompt,
    default_model_id: modelId,
    capabilities: capabilities.map((ref) => (typeof ref === "string" ? { ref } : ref)),
  });
  return created.id;
}
const session = async (agentId) => (await post("/sessions", { agent_id: agentId })).id;
const say = (id, text) =>
  post(`/sessions/${id}/messages`, { message: { role: "user", content: [{ type: "text", text }] } });
const events = async (id) => (await get(`/sessions/${id}/events?limit=1000`)).data;

/** Events of the turn `messageId` started, once it ends (or `wanted` appears). */
async function turnOf(id, messageId, { timeout = 240_000, until = "turn.completed" } = {}) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const all = await events(id);
    const started = all.find((e) => e.type === "turn.started" && e.data?.input_message_id === messageId);
    const turnId = started?.data?.turn_id;
    const turn = all.filter(
      (e) => e.context?.input_message_id === messageId || (turnId && e.context?.turn_id === turnId),
    );
    const types = turn.map((e) => e.type);
    if (types.includes(until)) return turn;
    if (until !== "turn.failed" && types.includes("turn.failed")) {
      const failed = turn.find((e) => e.type === "turn.failed");
      throw new Error(`turn failed: ${JSON.stringify(failed.data).slice(0, 300)}`);
    }
    await delay(1000);
  }
  throw new Error(`no ${until} for message ${messageId} in ${timeout / 1000}s`);
}
const tools = (turn) =>
  turn.filter((e) => e.type === "tool.completed").map((e) => ({ name: e.data?.tool_name, ok: e.data?.success }));
const succeeded = (turn, name) => tools(turn).some((t) => t.name === name && t.ok);

/** Every event the session's stream sends until `stop` resolves. */
function stream(id, stop) {
  const seen = [];
  const controller = new AbortController();
  const done = (async () => {
    const response = await fetch(`${apiBase}/sessions/${id}/sse`, {
      headers: { Authorization: `Bearer ${token}`, Accept: "text/event-stream" },
      signal: controller.signal,
    });
    assert(response.ok, `GET /sessions/${id}/sse: HTTP ${response.status}`);
    const decoder = new TextDecoder();
    let buffer = "";
    for await (const chunk of response.body) {
      buffer += decoder.decode(chunk, { stream: true });
      let end;
      while ((end = buffer.indexOf("\n\n")) >= 0) {
        const frame = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const type = frame.match(/^event: ?(.*)$/m)?.[1];
        if (type) seen.push(type);
      }
    }
  })().catch((error) => {
    if (error.name !== "AbortError") seen.push(`error: ${error.message}`);
  });
  stop.finally(() => controller.abort());
  return { seen, done };
}

// --- Rows -------------------------------------------------------------------

const rows = {
  /** A message streams a reply; `wait=true` returns the reply; cancel stops a turn. */
  async chat() {
    const id = await session(await agent("chat", "You are a helpful assistant. Keep answers short."));

    let finish;
    const ended = new Promise((done) => (finish = done));
    const live = stream(id, ended);
    await delay(1000);
    const first = await say(id, "Name three primary colors, one per line.");
    try {
      await turnOf(id, first.id);
    } finally {
      await delay(1000);
      finish();
      await live.done;
    }
    const deltas = live.seen.filter((t) => t === "output.message.delta").length;
    assert(deltas > 0, `the reply did not stream (stream saw: ${[...new Set(live.seen)].join(", ")})`);
    assert(live.seen.includes("turn.completed"), "the stream did not report the turn's end");

    const waited = await api(`/sessions/${id}/messages?wait=true&timeout_ms=180000`, "POST", {
      message: { role: "user", content: [{ type: "text", text: "What is 6 times 7?" }] },
    });
    assert.equal(waited.status, 200, `wait=true: HTTP ${waited.status}`);
    assert.equal(waited.body.status, "completed", JSON.stringify(waited.body).slice(0, 300));
    const replies = waited.body.messages.filter((m) => m.role === "agent");
    assert(
      replies.some((m) => m.content?.some((part) => part.type === "text" && part.text?.trim())),
      "wait=true returned no reply",
    );

    // Deltas are live only (the stored history has none), so the stream says
    // when the reply is under way.
    let stop;
    const watching = stream(id, new Promise((done) => (stop = done)));
    await delay(1000);
    const long = await say(id, "Write a 3000-word essay on the history of bridges, with ten sections.");
    try {
      const deadline = Date.now() + 120_000;
      while (!watching.seen.includes("output.message.delta")) {
        assert(Date.now() < deadline, `the long reply did not stream in 120s (saw: ${[...new Set(watching.seen)].join(", ")})`);
        await delay(500);
      }
    } finally {
      stop();
      await watching.done;
    }
    await post(`/sessions/${id}/cancel`);
    const cancelled = await turnOf(id, long.id, { until: "turn.cancelled", timeout: 60_000 });
    assert(!cancelled.some((e) => e.type === "turn.completed"), "the cancelled turn completed");
    return { streamed_deltas: deltas };
  },

  /** The agent writes and edits files, runs bash, and the session's diff lists the change. */
  async coding() {
    const id = await session(
      await agent(
        "coding",
        "You are a careful coding assistant. Use your file tools to change files and bash to check them. Do exactly what is asked, then reply in one sentence.",
        ["session_file_system", "bashkit_shell"],
      ),
    );
    const first = await say(
      id,
      "Create the file /workspace/notes/plan.md with exactly three lines: alpha, beta, gamma. Then run `wc -l /workspace/notes/plan.md` with bash.",
    );
    const one = await turnOf(id, first.id);
    assert(succeeded(one, "write_file"), `no file was written: ${JSON.stringify(tools(one))}`);
    assert(succeeded(one, "bash"), `bash did not run: ${JSON.stringify(tools(one))}`);
    const before = await post(`/sessions/${id}/git/commit`, { message: "plan" });

    const second = await say(
      id,
      "In /workspace/notes/plan.md replace the line beta with BETA, keeping the other lines. Then show the file with `cat` in bash.",
    );
    const two = await turnOf(id, second.id);
    assert(
      succeeded(two, "edit_file") || succeeded(two, "write_file"),
      `the file was not edited: ${JSON.stringify(tools(two))}`,
    );
    assert(succeeded(two, "bash"), `bash did not run: ${JSON.stringify(tools(two))}`);
    const after = await post(`/sessions/${id}/git/commit`, { message: "edit plan" });
    const diff = await get(`/sessions/${id}/git/diff?oid=${after.oid}&base=${before.oid}`);
    const changed = diff.entries.map((entry) => entry.path ?? entry.new_path ?? JSON.stringify(entry));
    assert(
      changed.some((path) => path.endsWith("notes/plan.md")),
      `the diff does not list the edited file: ${changed.join(", ")}`,
    );
    assert(diff.patch?.includes("+BETA"), "the diff does not show the edit");
    return { files_changed: changed.length };
  },

  /**
   * `start_thread` runs a child that asks its parent, gets the answer, and
   * calls `finish`; the parent's assignment succeeds.
   */
  async threads() {
    const parent = await session(
      await agent(
        "threads",
        [
          "You coordinate work through threads.",
          "When the person asks for the color check, call start_thread once, with title \"Color check\" and this brief:",
          "\"Call ask_parent to ask which color to use. When the answer arrives, call finish with a one-line summary that names the color.\"",
          "When a thread asks which color to use, answer it with message_thread: \"Use teal.\" Do not ask the person.",
        ].join(" "),
        ["threads"],
      ),
    );
    const asked = await say(parent, "Run the color check.");
    const first = await turnOf(parent, asked.id);
    assert(succeeded(first, "start_thread"), `no thread started: ${JSON.stringify(tools(first))}`);

    const deadline = Date.now() + 300_000;
    let tasks;
    while (Date.now() < deadline) {
      tasks = (await get(`/sessions/${parent}/tasks`)).data ?? [];
      const assignment = tasks.find((t) => t.kind === "assignment");
      if (assignment && ["succeeded", "failed", "cancelled"].includes(assignment.state)) break;
      await delay(2000);
    }
    const assignment = tasks?.find((t) => t.kind === "assignment");
    assert(assignment, `the parent has no assignment: ${JSON.stringify(tasks)}`);
    assert.equal(assignment.state, "succeeded", `assignment: ${JSON.stringify(assignment).slice(0, 400)}`);

    const thread = assignment.links?.child_session_id;
    const childTools = tools(await events(thread));
    assert(childTools.some((t) => t.name === "ask_parent" && t.ok), `the thread did not ask: ${JSON.stringify(childTools)}`);
    assert(childTools.some((t) => t.name === "finish" && t.ok), `the thread did not finish: ${JSON.stringify(childTools)}`);
    assert(!childTools.some((t) => t.name === "start_thread"), "the thread tried to start a thread");
    const parentTools = tools(await events(parent));
    assert(
      parentTools.some((t) => t.name === "message_thread" && t.ok),
      `the parent never answered the thread: ${JSON.stringify(parentTools)}`,
    );
    return { thread };
  },

  /** The parent spawns another agent by name; it runs as itself and its answer comes back. */
  async spawn() {
    const helper = await agent(
      "spawn-helper",
      "You are the archivist. Whatever you are asked, answer in one sentence that contains the word lighthouse.",
    );
    const parent = await session(
      await agent(
        "spawn",
        [
          "You delegate. When the person asks for the archivist, call spawn_agent with agent \"archivist\" and the person's question as the brief.",
          "Then reply with the archivist's answer, word for word.",
        ].join(" "),
        [{ ref: "spawn_agents", config: { self: false, agents: [{ agent_id: helper, name: "archivist" }] } }],
      ),
    );
    const asked = await say(parent, "Ask the archivist what guides ships at night.");
    const first = await turnOf(parent, asked.id, { timeout: 300_000 });
    assert(succeeded(first, "spawn_agent"), `no agent spawned: ${JSON.stringify(tools(first))}`);
    // The answer reaches the parent in this turn or a later one.
    const deadline = Date.now() + 300_000;
    while (Date.now() < deadline) {
      const all = await events(parent);
      const said = all
        .filter((e) => e.type === "output.message.completed")
        .map((e) => JSON.stringify(e.data?.message?.content ?? ""))
        .join(" ");
      if (/lighthouse/i.test(said)) return { spawned: true };
      await delay(2000);
    }
    throw new Error("the archivist's answer never reached the parent");
  },

  /** The agent schedules a one-off run a minute out, and it fires on time. */
  async schedules() {
    const id = await session(
      await agent(
        "schedules",
        "You manage schedules. When asked to schedule something, call create_schedule with exactly the time and text given, then confirm in one sentence.",
        ["session_schedule"],
      ),
    );
    const at = new Date(Date.now() + 75_000);
    at.setUTCSeconds(0, 0);
    at.setUTCMinutes(at.getUTCMinutes() + 1);
    const asked = await say(
      id,
      `Create a one-off schedule at ${at.toISOString()} (UTC) with the description "Say: tick".`,
    );
    const first = await turnOf(id, asked.id);
    assert(succeeded(first, "create_schedule"), `no schedule created: ${JSON.stringify(tools(first))}`);
    const listed = (await get(`/sessions/${id}/schedules`)).data ?? [];
    assert(listed.length > 0, "the session lists no schedule");
    const deadline = at.getTime() + 120_000;
    while (Date.now() < deadline) {
      const turns = (await events(id)).filter((e) => e.type === "turn.completed");
      if (turns.length >= 2) {
        const lateSeconds = Math.round((Date.parse(turns[1].ts) - at.getTime()) / 1000);
        return { schedules: listed.length, fired_late_s: lateSeconds };
      }
      await delay(2000);
    }
    throw new Error(`the schedule for ${at.toISOString()} did not run within 2 minutes`);
  },

  /** A guarded tool waits for approval and runs once approved; auto mode runs a harmless call without asking. */
  async approvals() {
    const ask = "Run `echo approved-42` with bash, then reply with its output.";
    const guarded = await session(
      await agent("approvals", "You run shell commands with bash when asked.", [
        "bashkit_shell",
        { ref: "tool_approval", config: { mode: "protective" } },
      ]),
    );
    const sent = await say(guarded, ask);
    let requested;
    const deadline = Date.now() + 180_000;
    while (!requested && Date.now() < deadline) {
      const all = await events(guarded);
      const done = new Set(all.filter((e) => e.type === "tool.completed").map((e) => e.data?.tool_call_id));
      requested = all
        .filter((e) => e.type === "tool.call_requested")
        .flatMap((e) => e.data?.tool_calls ?? [])
        .find((call) => call.name === "approve_tool_call" && !done.has(call.id));
      if (!requested) await delay(1000);
    }
    assert(requested, "the guarded call never asked for approval");
    assert(
      !(await events(guarded)).some((e) => e.type === "tool.completed" && e.data?.tool_name === "bash"),
      "bash ran before it was approved",
    );
    for (let tries = 0; ; tries++) {
      const answer = await api(`/sessions/${guarded}/tool-approvals`, "POST", {
        decisions: [{ tool_call_id: requested.id, decision: "allow" }],
      }, { ok: false });
      if (answer.status < 300) break;
      // The request lands before the session waits on it.
      assert(answer.status === 409 && tries < 100, `tool-approvals: HTTP ${answer.status}`);
      await delay(200);
    }
    const resumed = await turnOf(guarded, sent.id);
    assert(succeeded(resumed, "bash"), `bash did not run after approval: ${JSON.stringify(tools(resumed))}`);

    const auto = await session(
      await agent("approvals-auto", "You run shell commands with bash when asked.", [
        "bashkit_shell",
        { ref: "tool_approval", config: { mode: "auto" } },
      ]),
    );
    const autoTurn = await turnOf(auto, (await say(auto, ask)).id);
    assert(succeeded(autoTurn, "bash"), `auto mode did not run bash: ${JSON.stringify(tools(autoTurn))}`);
    const asked = (await events(auto))
      .filter((e) => e.type === "tool.call_requested")
      .flatMap((e) => e.data?.tool_calls ?? [])
      .some((call) => call.name === "approve_tool_call");
    assert(!asked, "auto mode asked a person about a harmless command");
    return { approved: requested.id };
  },

  /** A fact remembered in one session is recalled in the next; a session database is written and queried. */
  async memory() {
    const remembering = await agent(
      "memory",
      "You keep notes. Save facts the person asks you to remember with your memory tools, and look them up when asked. Use the SQL tools for databases.",
      ["agent_memory", "session_sql_database"],
    );
    const first = await session(remembering);
    const saved = await turnOf(first, (await say(first, "Remember this: the project codename is Heron.")).id);
    assert(tools(saved).some((t) => t.ok), `nothing was saved: ${JSON.stringify(tools(saved))}`);
    const second = await session(remembering);
    const recalled = await post(`/sessions/${second}/messages?wait=true&timeout_ms=240000`, {
      message: { role: "user", content: [{ type: "text", text: "What is the project codename? Answer with the name only." }] },
    });
    const reply = JSON.stringify((recalled.messages ?? []).filter((m) => m.role === "agent"));
    assert(/heron/i.test(reply), `the next session did not recall the fact: ${reply.slice(0, 300)}`);

    const sql = await turnOf(
      second,
      (await say(second, "Create a table colors(name text), insert red and blue, then query how many rows it has.")).id,
    );
    assert(succeeded(sql, "sql_execute"), `no SQL write: ${JSON.stringify(tools(sql))}`);
    assert(succeeded(sql, "sql_query"), `no SQL query: ${JSON.stringify(tools(sql))}`);
    return { recalled: true };
  },

  /** A turn's usage reaches the control plane; a session over its budget is refused. */
  async spend() {
    const id = await session(await agent("spend", "You are a helpful assistant. Keep answers short."));
    await turnOf(id, (await say(id, "Say hello.")).id);
    let cost;
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      cost = await get(`/sessions/${id}/cost`);
      if ((cost.model_calls ?? 0) > 0) break;
      await delay(2000);
    }
    assert((cost?.model_calls ?? 0) > 0, `no usage reached the control plane: ${JSON.stringify(cost).slice(0, 300)}`);

    const capped = await session(await agent("spend-capped", "You are a helpful assistant. Keep answers short."));
    await post("/budgets", { subject_type: "session", subject_id: capped, currency: "tokens", limit: 1 });
    const refused = await api(`/sessions/${capped}/messages`, "POST", {
      message: { role: "user", content: [{ type: "text", text: "Write a long poem." }] },
    }, { ok: false });
    if (refused.status < 300) {
      const turn = await turnOf(capped, refused.body.id, { until: "turn.failed", timeout: 120_000 });
      assert(!turn.some((e) => e.type === "turn.completed"), "a session over its budget completed a turn");
    }
    return { model_calls: cost.model_calls, over_budget_status: refused.status };
  },
};

// --- Run --------------------------------------------------------------------

const wanted = process.argv.slice(2).filter((arg) => !arg.startsWith("-"));
const selected = wanted.length ? wanted : Object.keys(rows);
for (const name of selected) assert(rows[name], `unknown row ${name}`);

const report = { tag, model: modelSlug, started_at: new Date().toISOString(), rows: [] };
let setupError;
try {
  const password = await startStack();
  await signIn(password);
  modelId = await chooseModel();
} catch (error) {
  setupError = error;
}
for (const name of selected) {
  const attempts = [];
  for (let attempt = 1; attempt <= 2 && !setupError; attempt++) {
    const started = Date.now();
    try {
      const detail = await rows[name]();
      attempts.push({ ok: true, seconds: Math.round((Date.now() - started) / 1000), detail });
      break;
    } catch (error) {
      attempts.push({ ok: false, seconds: Math.round((Date.now() - started) / 1000), error: redact(error.message) });
    }
  }
  if (setupError) attempts.push({ ok: false, error: `setup: ${redact(setupError.message)}` });
  const ok = attempts.at(-1).ok;
  report.rows.push({ name, ok, attempts });
  log(`${ok ? "PASS" : "FAIL"} ${name} ${JSON.stringify(attempts)}`);
}
if (!setupError) {
  try {
    const sql = async (query) =>
      (await compose(["exec", "-T", "postgres", "psql", "-U", "everruns", "-d", "everruns", "-tAc", query], { quiet: true })).trim();
    const placed = Number(await sql("SELECT count(*) FROM dp_orgs"));
    const v1 = Number(await sql("SELECT count(*) FROM sessions"));
    const ok = placed >= 1 && v1 === 0;
    report.rows.push({ name: "ran on v2", ok, attempts: [{ ok, detail: { placed_orgs: placed, v1_sessions: v1 } }] });
  } catch (error) {
    report.rows.push({ name: "ran on v2", ok: false, attempts: [{ ok: false, error: redact(error.message) }] });
  }
}
await stopStack();

report.finished_at = new Date().toISOString();
if (process.env.E2E_REPORT) await writeFile(process.env.E2E_REPORT, `${JSON.stringify(report, null, 2)}\n`);
if (process.env.GITHUB_STEP_SUMMARY) {
  const lines = [
    `### v2 workflows (images \`${tag}\`, model \`${modelSlug}\`)`,
    "",
    "| Workflow | Result | Attempts |",
    "|---|---|---|",
    ...report.rows.map(
      (row) =>
        `| ${row.name} | ${row.ok ? "pass" : "**fail**"} | ${row.attempts
          .map((a) => (a.ok ? `ok${a.seconds ? ` ${a.seconds}s` : ""}` : `failed: ${String(a.error).replaceAll("|", "/").slice(0, 200)}`))
          .join("; ")} |`,
    ),
    "",
  ];
  await appendFile(process.env.GITHUB_STEP_SUMMARY, lines.join("\n"));
}
process.exit(report.rows.every((row) => row.ok) ? 0 : 1);
