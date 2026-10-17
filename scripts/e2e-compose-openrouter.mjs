#!/usr/bin/env node
// Opt-in, paid E2E: published Compose images, browser onboarding, and real inference.
// Keep credentials out of subprocess arguments, logs, traces, HARs, and saved browser state.
import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const setupOnly = process.argv.includes("--setup-only");
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
if (process.argv.includes("--help")) {
  console.log(`Usage: OPENROUTER_API_KEY=... EVERRUNS_E2E_ALLOW_PROVIDER_STORAGE=1 node scripts/e2e-compose-openrouter.mjs
--setup-only verifies clean Compose startup and UI login without any provider credential.
Requires Docker Compose and apps/ui dependencies (pnpm install, pnpm exec playwright install chromium).
Uses a unique project, loopback ports, disposable volumes, and an isolated browser.
The opt-in permits application startup to encrypt the existing environment key into this disposable database.
No raw provider key is typed or submitted through browser UI/API.
Optional: EVERRUNS_TAG (default latest), COMPOSE_E2E_FILE (default download from main),
COMPOSE_E2E_OUTPUT (default temporary directory), OPENROUTER_MODEL (default openai/gpt-6-luna).
Always tears down its project and volumes. Video and a redacted report remain in the output directory.
Never run with shell tracing, a management key, or a production Compose project.`);
  process.exit(0);
}
if (!setupOnly) {
  assert(
    process.env.OPENROUTER_API_KEY,
    "OPENROUTER_API_KEY is required; no simulated provider fallback",
  );
  assert.equal(
    process.env.EVERRUNS_E2E_ALLOW_PROVIDER_STORAGE,
    "1",
    "Explicit consent to disposable provider credential storage is required",
  );
}
const key = process.env.OPENROUTER_API_KEY;
const modelSlug = process.env.OPENROUTER_MODEL || "openai/gpt-6-luna";
const require = createRequire(join(root, "apps/ui/package.json"));
const { chromium, expect } = require("@playwright/test");
let output;
if (process.env.COMPOSE_E2E_OUTPUT) {
  const parent = resolve(process.env.COMPOSE_E2E_OUTPUT);
  await mkdir(parent, { recursive: true });
  output = await mkdtemp(join(parent, "run-"));
} else {
  output = await mkdtemp(join(tmpdir(), "everruns-compose-e2e-"));
}
const work = await mkdtemp(join(tmpdir(), "everruns-compose-stack-"));
const project = `everruns-e2e-${randomBytes(6).toString("hex")}`;
const report = { project, started_at: new Date().toISOString(), model: modelSlug, steps: [] };
let browser,
  context,
  page,
  terminal,
  sessionId,
  stackStarted = false;
let activeCommand;
const controller = new AbortController();
let commandOutput = "";
const redact = (value) => (key ? String(value).replaceAll(key, "[REDACTED]") : String(value));
const log = (message) => {
  const text = redact(message);
  console.log(text);
  report.steps.push(text);
  commandOutput += `${text}\n`;
};
const env = Object.fromEntries(
  Object.entries(process.env).filter(([name]) =>
    ["PATH", "HOME", "DOCKER_CONTEXT", "DOCKER_HOST", "DOCKER_CONFIG", "TMPDIR"].includes(name),
  ),
);
Object.assign(env, {
  COMPOSE_PROJECT_NAME: project,
  EVERRUNS_TAG: process.env.EVERRUNS_TAG || "latest",
});

const command = (args, timeout) => docker(["compose", ...args], timeout);
async function docker(args, timeout = 180_000) {
  log(`$ docker ${args.join(" ")}`);
  return await new Promise((resolveCommand, reject) => {
    const child = spawn("docker", args, { cwd: work, env });
    activeCommand = child;
    let result = "";
    const onData = (chunk) => {
      const text = redact(chunk);
      result += text;
      commandOutput += text;
    };
    child.stdout.on("data", onData);
    child.stderr.on("data", onData);
    const timer = setTimeout(() => {
      child.kill("SIGTERM");
    }, timeout);
    child.on("error", reject);
    child.on("close", (code) => {
      clearTimeout(timer);
      activeCommand = undefined;
      code === 0
        ? resolveCommand(result)
        : reject(new Error(`docker ${args[0]} failed (exit ${code}); see commands.log`));
    });
  });
}
async function freePort() {
  const server = createServer();
  await new Promise((res, rej) => {
    server.once("error", rej);
    server.listen(0, "127.0.0.1", res);
  });
  const port = server.address().port;
  await new Promise((res) => server.close(res));
  return port;
}
const delay = (ms) => new Promise((res) => setTimeout(res, ms));
async function api(path, method = "GET", data) {
  let response;
  try {
    response = await context.request.fetch(`${report.base_url}/api/v1${path}`, {
      method,
      data,
      timeout: 30_000,
    });
  } catch {
    // Playwright request errors may include cookie headers in their diagnostic log.
    throw new Error(`${method} ${path}: request failed`);
  }
  assert(response.ok(), `${method} ${path}: HTTP ${response.status()}`);
  return response.status() === 204 ? undefined : response.json();
}
async function waitTurn(id) {
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    controller.signal.throwIfAborted();
    const { data: events } = await api(`/sessions/${id}/events?limit=1000`);
    const types = events.map((event) => event.type);
    assert(
      !types.includes("turn.failed"),
      "Real provider turn failed; inspect the isolated session",
    );
    assert(!types.includes("turn.cancelled"), "Real provider turn was cancelled");
    // A runaway tool loop must not spend indefinitely. Cancellation also runs on timeout/failure.
    assert(
      types.filter((type) => type === "llm.generation").length <= 12,
      "Provider call budget exceeded",
    );
    if (types.includes("turn.completed")) {
      assert(types.includes("output.message.completed"), "Turn produced no assistant output");
      return events;
    }
    await delay(1000);
  }
  throw new Error("Real provider turn exceeded 180 seconds");
}
async function send(text) {
  const input = page.locator("textarea");
  await expect(input).toBeEditable();
  const submitted = page.waitForResponse(
    (r) =>
      /\/api\/v1\/sessions\/[^/]+\/messages$/.test(new URL(r.url()).pathname) &&
      r.request().method() === "POST",
  );
  await input.fill(text);
  await input.press("Enter");
  const response = await submitted;
  assert(response.ok(), `Sending chat message: HTTP ${response.status()}`);
  sessionId = new URL(response.url()).pathname.split("/").at(-2);
  return waitTurn(sessionId);
}

const abort = () => {
  controller.abort(new Error("E2E interrupted"));
  activeCommand?.kill("SIGTERM");
  void page?.close().catch(() => {});
};
process.once("SIGINT", abort);
process.once("SIGTERM", abort);
let refresh;
try {
  const port = await freePort();
  let metricsPort = await freePort();
  while (metricsPort === port) metricsPort = await freePort();
  report.base_url = `http://localhost:${port}`;
  env.EXAMPLE_PROXY_PORT = `127.0.0.1:${port}`;
  env.VICTORIAMETRICS_PORT = `127.0.0.1:${metricsPort}`;
  log(`Evidence: ${output}`);
  log(`Isolated Compose project: ${project}; UI ${report.base_url}`);
  browser = await chromium.launch({ headless: true, env });
  context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    recordVideo: { dir: join(output, "video"), size: { width: 1280, height: 800 } },
  });
  page = await context.newPage();
  page.setDefaultTimeout(30_000);
  await page.setContent(
    "<html><style>body{background:#101820;color:#e8f3ed;font:18px monospace;padding:24px}h1{font:28px sans-serif;color:#9cdec2}pre{white-space:pre-wrap}</style><h1>Everruns OSS · real OpenRouter E2E</h1><p>Real execution. Generated secrets excluded; provider input stays masked. No trace or HAR recording.</p><pre></pre></html>",
  );
  terminal = true;
  refresh = setInterval(() => {
    if (terminal)
      void page
        .locator("pre")
        .evaluate((el, text) => {
          el.textContent = text;
          window.scrollTo(0, document.body.scrollHeight);
        }, commandOutput.slice(-6000))
        .catch(() => {});
  }, 250);
  report.compose_source =
    process.env.COMPOSE_E2E_FILE ||
    "https://raw.githubusercontent.com/everruns/everruns/main/examples/docker-compose-full.yaml";
  log(`Download Compose: ${report.compose_source}`);
  const compose = process.env.COMPOSE_E2E_FILE
    ? await readFile(resolve(process.env.COMPOSE_E2E_FILE))
    : Buffer.from(
        await (
          await fetch(report.compose_source, {
            signal: AbortSignal.any([controller.signal, AbortSignal.timeout(30_000)]),
          })
        ).arrayBuffer(),
      );
  assert(compose.includes("services:"), "Compose download is not a Compose document");
  report.compose_sha256 = createHash("sha256").update(compose).digest("hex");
  await writeFile(join(work, "docker-compose.yaml"), compose);
  const password = randomBytes(16).toString("hex");
  const secrets = [
    `SECRETS_ENCRYPTION_KEY=kek-v1:${randomBytes(32).toString("base64")}`,
    `AUTH_JWT_SECRET=${randomBytes(32).toString("hex")}`,
    `WORKER_GRPC_AUTH_TOKEN=${randomBytes(32).toString("hex")}`,
    "AUTH_ADMIN_EMAIL=admin@example.com",
    `AUTH_ADMIN_PASSWORD=${password}`,
    "",
  ].join("\n");
  await writeFile(join(work, ".env"), secrets, { mode: 0o600 });
  log("Generated private .env with encryption, JWT, worker, and admin secrets (values excluded)");
  if (!setupOnly) {
    env.DEFAULT_OPENROUTER_API_KEY = key;
    env.SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV = "true";
    log(
      "Existing inference key supplied privately to server startup; the app encrypts it into the default organization (no raw-key UI/API submission)",
    );
  }
  report.credential_mode = setupOnly ? "none" : "startup_materialization";
  report.images = (await command(["config", "--images"])).trim().split("\n");
  await command(["pull", "--quiet"], 600_000);
  report.engine_version = (await docker(["version", "--format", "{{.Server.Version}}"])).trim();
  report.compose_version = (await command(["version", "--short"])).trim();
  report.image_metadata = (
    await docker([
      "image",
      "inspect",
      ...new Set(report.images),
      "--format",
      '{"id":{{json .Id}},"digests":{{json .RepoDigests}},"architecture":{{json .Architecture}},"labels":{{json (index .Config "Labels")}}}',
    ])
  )
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  stackStarted = true;
  await command(["up", "-d", "--wait", "--wait-timeout", "180"], 210_000);
  report.services = JSON.parse(
    `[${(await command(["ps", "--format", "json"])).trim().split("\n").join(",")}]`,
  );
  assert.equal(report.services.length, 10, "Expected all ten documented Compose services");
  for (const service of report.services) {
    assert.equal(service.State, "running", `${service.Service} is not running`);
    if (service.Health)
      assert.equal(service.Health, "healthy", `${service.Service} is not healthy`);
  }
  const healthResponse = await fetch(`${report.base_url}/health`, {
    signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
  });
  assert(healthResponse.ok, "Proxy health request failed");
  report.health = await healthResponse.json();
  log(`All ten services running; health: ${JSON.stringify(report.health)}`);
  if (!setupOnly) {
    log("OpenRouter preflight: one real request, max_tokens=32");
    const preflight = await fetch("https://openrouter.ai/api/v1/chat/completions", {
      method: "POST",
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(45_000)]),
      headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        model: modelSlug,
        max_tokens: 32,
        reasoning: { effort: "minimal" },
        messages: [{ role: "user", content: "What is 2 + 2? Reply with the number only." }],
      }),
    });
    assert(preflight.ok, `OpenRouter preflight: HTTP ${preflight.status}`);
    const probe = await preflight.json();
    assert.equal(probe.choices[0].message.content.trim(), "4");
    report.preflight = { model: probe.model, answer: "4", usage: probe.usage };
    log(`Real OpenRouter answer: 4; ${JSON.stringify(probe.usage)}`);
  }
  terminal = false;
  clearInterval(refresh);
  await page.goto(`${report.base_url}/login`);
  await page.getByLabel("Email", { exact: true }).fill("admin@example.com");
  await page.getByRole("button", { name: "Continue with email" }).click();
  const passwordInput = page.getByLabel("Password", { exact: true });
  await expect(passwordInput).toHaveAttribute("type", "password");
  await passwordInput.fill(password);
  await page.getByRole("button", { name: "Continue", exact: true }).click();
  await expect(page).toHaveURL(/\/chats$/);
  if (setupOnly) {
    await expect(page.getByText("No intelligence available", { exact: true })).toBeVisible();
  }
  log("UI login passed");
  if (setupOnly) {
    report.result = "setup_verified";
    log("SETUP VERIFIED: real provider storage, chat creation, and agent run were not attempted");
  } else {
    await page.goto(`${report.base_url}/models/providers`);
    const card = page
      .locator('[data-slot="card"]')
      .filter({ has: page.getByRole("link", { name: "OpenRouter", exact: true }) });
    await expect(card.getByText("Configured", { exact: true })).toBeVisible();
    log("UI shows the provider configured by application startup; secret entry is excluded");
    const providers = (await api("/providers")).data;
    const provider = providers.find((p) => p.provider_type === "openrouter");
    assert(provider?.api_key_set, "OpenRouter credential was not persisted");
    await api(`/providers/${provider.id}/sync-models`, "POST", {});
    let models = (await api("/models")).data;
    let model = models.find((m) => m.provider_id === provider.id && m.model_id === modelSlug);
    if (!model)
      model = await api(`/providers/${provider.id}/models`, "POST", {
        model_id: modelSlug,
        display_name: "Compose E2E GPT-6 Luna",
        capabilities: ["chat", "tools"],
        enabled: true,
      });
    await api(`/models/${model.id}`, "PATCH", { enabled: true });
    const me = await api("/auth/me");
    await api(`/orgs/${me.organizations[0].public_id}`, "PATCH", { default_model_id: model.id });
    report.provider_id = provider.id;
    report.model_id = model.id;
    log(`Selected bounded-cost model ${modelSlug} as the organization default`);
    await page.goto(`${report.base_url}/chats`);
    const chat = await api("/sessions/platform-chat", "POST", {});
    await api(`/sessions/${chat.id}`, "PATCH", { model_id: model.id });
    await page.reload();
    const name = `compose-math-${randomBytes(3).toString("hex")}`;
    const events = await send(
      `Create one active agent now using your agent management tools. Name (slug): ${name}. Display name: Compose Math. System prompt: You are a concise math tutor. Answer arithmetic questions with the result and one short explanation. Default model ID: ${model.id}. Use the generic harness and no extra capabilities or tools. Persist the agent, then give me its link. Do not only describe how to create it. No questions or extra agents.`,
    );
    report.creation_event_types = [...new Set(events.map((event) => event.type))];
    assert(
      report.creation_event_types.includes("tool.completed"),
      "Chat never completed a tool call",
    );
    assert(
      report.creation_event_types.includes("llm.generation"),
      "Chat never called the provider",
    );
    const agents = (await api("/agents?limit=100")).data;
    const agent = agents.find((a) => a.name === name);
    assert(agent, "Chat claimed success but no persisted agent exists");
    const saved = await api(`/agents/${agent.id}`);
    assert.equal(saved.status, "active");
    assert.match(saved.system_prompt, /math tutor/i);
    assert.equal(saved.default_model_id, model.id);
    report.agent_id = agent.id;
    log(`Chat created persisted active agent: ${agent.id} (${name})`);
    await expect(
      page.getByRole("link", { name: /Compose Math|compose-math/ }).first(),
    ).toBeVisible();
    await page.goto(`${report.base_url}/agents/${agent.id}`);
    await expect(page.getByText(name, { exact: true }).first()).toBeVisible();
    await page.screenshot({ path: join(output, "persisted-agent.png") });
    await page.goto(`${report.base_url}/playground/new?agent=${agent.id}`);
    await page.getByRole("button", { name: "Start Playground chat", exact: true }).click();
    await expect(page).toHaveURL(/\/playground\/session_/);
    const runEvents = await send(
      "What is 7 multiplied by 8? Include the result and a short explanation.",
    );
    const run = await api(`/sessions/${sessionId}`);
    assert.equal(run.agent_id, agent.id, "Successful run belongs to a different agent");
    assert.equal(run.model_id ?? saved.default_model_id, model.id, "Run used a different model");
    const messages = (await api(`/sessions/${sessionId}/messages?limit=100`)).data;
    const answer = messages
      .filter((m) => m.role === "agent" || m.role === "assistant")
      .flatMap((m) => m.content ?? [])
      .filter((part) => part.type === "text")
      .map((part) => part.text)
      .join("\n");
    assert.match(answer, /\b56\b/, "Persisted agent did not return the correct arithmetic answer");
    await expect(page.getByText(/56/).last()).toBeVisible();
    await page.screenshot({ path: join(output, "successful-run.png") });
    report.session_id = sessionId;
    report.answer = answer;
    report.run_event_types = [...new Set(runEvents.map((event) => event.type))];
    report.result = "passed";
    log(
      `PASS: persisted agent ${agent.id}, real successful session ${sessionId}, answer ${answer}`,
    );
  }
  await delay(2000);
} catch (error) {
  report.result = "failed";
  report.error = redact(error.message);
  log(`FAIL: ${report.error}`);
  process.exitCode = 1;
} finally {
  clearInterval(refresh);
  if (sessionId && context) {
    try {
      await api(`/sessions/${sessionId}/cancel`, "POST", {});
    } catch {}
  }
  for (const resource of [context, browser]) {
    try {
      await resource?.close();
    } catch {
      report.recording_error = "Browser recording failed to close cleanly";
      process.exitCode = 1;
    }
  }
  if (stackStarted) {
    try {
      await command(["down", "-v", "--remove-orphans"], 120_000);
    } catch (error) {
      report.cleanup_error = redact(error.message);
      process.exitCode = 1;
    }
  }
  // Retained encrypted records still depend on their encryption key after failed teardown.
  if (report.cleanup_error) report.retained_stack_directory = work;
  else await rm(work, { recursive: true, force: true });
  report.finished_at = new Date().toISOString();
  await writeFile(join(output, "report.json"), JSON.stringify(report, null, 2));
  await writeFile(join(output, "commands.log"), commandOutput);
  console.log(`Evidence: ${output}`);
}
