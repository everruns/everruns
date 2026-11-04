import assert from "node:assert/strict";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, writeFile, rm, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
const root = dirname(fileURLToPath(import.meta.url));
const frame = (value) => `data: ${JSON.stringify(value)}\n\n`;
const calls = [];
const waiting = new Map();
let leak = false;
const model = createServer(async (req, res) => {
  try {
    if (req.url === "/empty") {
      res.writeHead(204);
      res.end();
      return;
    }
    if (req.url.startsWith("/ack/")) {
      const key = req.url.slice(5);
      assert.ok(waiting.has(key), `no stream waiting for ${key}`);
      waiting.get(key)();
      waiting.delete(key);
      res.end("ok");
      return;
    }
    if (req.url === "/credentials") {
      leak = true;
      res.end("leaked");
      return;
    }
    const [driver, scenario] = req.url.split("/").slice(1);
    let raw = "";
    for await (const chunk of req) raw += chunk;
    const body = JSON.parse(raw);
    calls.push({ driver, scenario, body });
    assert.equal(req.headers.authorization, "Bearer test-key");
    assert.equal(req.headers["x-test"], "call");
    assert.equal(req.headers["content-type"], "application/json");
    if (driver === "openrouter") {
      assert.deepEqual(body.models, ["test-model", "fallback-model"]);
      assert.equal(body.route, "fallback");
      assert.equal(req.headers["x-title"], "WASM transport test");
    }
    if (scenario === "unauthorized") {
      res.writeHead(401, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          error: { code: "invalid_api_key", message: "bad key" },
        }),
      );
      return;
    }
    if (scenario === "too-large") {
      res.writeHead(400, { "content-type": "application/json" });
      res.end(
        JSON.stringify({
          error: {
            code: "context_length_exceeded",
            message: "context length exceeded",
          },
        }),
      );
      return;
    }
    if (scenario === "redirect") {
      res.writeHead(302, {
        location: `http://127.0.0.1:${model.address().port}/credentials`,
      });
      res.end();
      return;
    }
    if (scenario === "timeout") {
      res.writeHead(200, { "content-type": "text/event-stream" });
      res.flushHeaders();
      return; // the WASM deadline must abort the outstanding fetch/reader
    }
    if (scenario === "malformed") {
      res.end("{not-json");
      return;
    }
    const completions = req.url.endsWith("/chat/completions");
    if (scenario === "non-streaming") {
      res.setHeader("content-type", "application/json");
      if (body.stream) {
        res.setHeader("content-type", "text/event-stream");
        res.end(
          frame({ type: "response.output_text.delta", delta: "42 ✓" }) +
            frame({
              type: "response.completed",
              response: { id: "reply", status: "completed", output: [] },
            }),
        );
        return;
      }
      res.end(
        JSON.stringify(
          completions
            ? {
                id: "reply",
                choices: [
                  {
                    message: { role: "assistant", content: "42 ✓" },
                    finish_reason: "stop",
                  },
                ],
                usage: {
                  prompt_tokens: 10,
                  completion_tokens: 4,
                  total_tokens: 14,
                },
              }
            : {
                id: "reply",
                status: "completed",
                output: [
                  {
                    type: "message",
                    role: "assistant",
                    content: [{ type: "output_text", text: "42 ✓" }],
                  },
                ],
                usage: { input_tokens: 10, output_tokens: 4, total_tokens: 14 },
              },
        ),
      );
      return;
    }
    if (
      scenario === "retry" &&
      calls.filter((c) => c.driver === driver && c.scenario === scenario)
        .length %
        3 ===
        1
    ) {
      res.writeHead(429, {
        "retry-after": "0",
        "content-type": "application/json",
      });
      res.end(
        JSON.stringify({
          error: { code: "rate_limit_exceeded", message: "rate limit" },
        }),
      );
      return;
    }
    assert.equal(body.stream, true);
    assert.ok(Math.abs(body.temperature - 0.2) < 1e-6);
    const tool = completions ? body.tools[0].function : body.tools[0];
    assert.equal(tool.name, "add");
    const result = completions
      ? body.messages.find((m) => m.role === "tool")
      : body.input.find((m) => m.type === "function_call_output");
    const call = {
      type: "function_call",
      id: "fc_1",
      call_id: "call_1",
      name: "add",
      arguments: '{"a":20,"b":22}',
      status: "completed",
    };
    res.setHeader("content-type", "text/event-stream");
    let head, tail;
    if (!result) {
      if (completions) {
        head = frame({
          id: "reply",
          choices: [
            {
              delta: {
                tool_calls: [
                  {
                    index: 0,
                    id: "call_1",
                    type: "function",
                    function: { name: "add", arguments: '{"a":20,' },
                  },
                ],
              },
            },
          ],
        });
        tail =
          frame({
            choices: [
              {
                delta: {
                  tool_calls: [
                    { index: 0, function: { arguments: '"b":22}' } },
                  ],
                },
                finish_reason: "tool_calls",
              },
            ],
          }) + "data: [DONE]\n\n";
      } else {
        head = frame({
          type: "response.output_item.added",
          output_index: 0,
          item: { ...call, arguments: "", status: "in_progress" },
        });
        tail =
          frame({
            type: "response.function_call_arguments.delta",
            item_id: "fc_1",
            delta: call.arguments,
          }) +
          frame({
            type: "response.output_item.done",
            output_index: 0,
            item: call,
          }) +
          frame({
            type: "response.completed",
            response: {
              id: "reply",
              status: "completed",
              output: [call],
              usage: { input_tokens: 10, output_tokens: 4, total_tokens: 14 },
            },
          });
      }
    } else {
      assert.equal(completions ? result.content : result.output, "42");
      assert.equal(
        completions ? result.tool_call_id : result.call_id,
        "call_1",
      );
      const replay = completions
        ? body.messages.find((m) => m.tool_calls)?.tool_calls[0]
        : body.input.find((m) => m.type === "function_call");
      assert.ok(replay, "assistant call must be replayed with its result");
      head = completions
        ? frame({ id: "reply2", choices: [{ delta: { content: "42 ✓" } }] })
        : frame({ type: "response.output_text.delta", delta: "42 ✓" });
      tail = completions
        ? frame({
            choices: [{ delta: {}, finish_reason: "stop" }],
            usage: {
              prompt_tokens: 10,
              completion_tokens: 4,
              total_tokens: 14,
            },
          }) + "data: [DONE]\n\n"
        : frame({
            type: "response.completed",
            response: {
              id: "reply2",
              status: "completed",
              output: [],
              usage: { input_tokens: 10, output_tokens: 4, total_tokens: 14 },
            },
          });
    }
    const acknowledged =
      result &&
      new Promise((resolve) => waiting.set(`${driver}/${scenario}`, resolve));
    // Split at every byte, including in the UTF-8 checkmark and JSON arguments.
    for (const byte of Buffer.from(head)) res.write(Buffer.from([byte]));
    if (acknowledged)
      await Promise.race([
        acknowledged,
        delay(5000).then(() => {
          throw Error("adapter buffered the stream");
        }),
      ]);
    res.end(tail);
  } catch (error) {
    res.destroy(error);
    model.failure = error;
  }
});
await new Promise((resolve) => model.listen(0, "127.0.0.1", resolve));
const reserve = createServer();
await new Promise((resolve) => reserve.listen(0, "127.0.0.1", resolve));
const port = reserve.address().port;
await new Promise((resolve) => reserve.close(resolve));
const dir = await mkdtemp(join(tmpdir(), "everruns-wasm-"));
let child;
try {
  const build = join(root, "build");
  const modules = [
    { name: "worker.mjs", esModule: join(root, "worker.mjs") },
    { name: "worker/shim.mjs", esModule: join(build, "worker/shim.mjs") },
  ];
  for (const name of await readdir(build)) {
    if (name.endsWith(".mjs") || name.endsWith(".js"))
      modules.push({ name, esModule: join(build, name) });
    if (name.endsWith(".wasm")) modules.push({ name, wasm: join(build, name) });
  }
  const config = `using Workerd = import "/workerd/workerd.capnp";
const config :Workerd.Config = (
  services = [
    (name = "main", worker = (
      modules = [${modules.map((m) => `(name = ${JSON.stringify(m.name)}, ${m.wasm ? "wasm" : "esModule"} = embed ${JSON.stringify(m.wasm || m.esModule)})`).join(",")}],
      compatibilityDate = "2026-10-16", globalOutbound = "internet",
      bindings = [(name = "TEST", durableObjectNamespace = (className = "DriverObject"))],
      durableObjectNamespaces = [(className = "DriverObject", uniqueKey = "provider-test")],
      durableObjectStorage = (inMemory = void)
    )),
    (name = "internet", network = (allow = ["local"]))
  ],
  sockets = [(name = "http", address = "127.0.0.1:${port}", http = (), service = "main")]
);`;
  await writeFile(join(dir, "config.capnp"), config);
  child = spawn(
    join(root, "node_modules/.bin/workerd"),
    ["serve", "-I", "/", join(dir, "config.capnp")],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  let logs = "";
  child.stdout.on("data", (data) => (logs += data));
  child.stderr.on("data", (data) => (logs += data));
  const base = `http://127.0.0.1:${model.address().port}`;
  let ready = false;
  for (let i = 0; i < 100; i++) {
    if (child.exitCode !== null) throw Error(logs);
    try {
      await fetch(`http://127.0.0.1:${port}/health`);
      ready = true;
      break;
    } catch {}
    await delay(50);
  }
  assert.ok(ready, logs);
  for (const where of ["worker", "do"]) {
    const empty = await fetch(
      `http://127.0.0.1:${port}/${where}?${new URLSearchParams({ base, driver: "completions", scenario: "empty-body" })}`,
      { signal: AbortSignal.timeout(15000) },
    );
    assert.equal(empty.status, 200, await empty.clone().text());
    assert.deepEqual(await empty.json(), { empty: true });
    console.log(`ok ${where}/empty-body`);
    for (const driver of [
      "completions",
      "responses",
      "openai-completions",
      "openai",
      "openrouter",
    ]) {
      for (const scenario of [
        "roundtrip",
        "retry",
        "non-streaming",
        "unauthorized",
        "too-large",
        "malformed",
        "timeout",
        "redirect",
      ]) {
        const before = calls.length;
        const response = await fetch(
          `http://127.0.0.1:${port}/${where}?${new URLSearchParams({ base, driver, scenario })}`,
          { signal: AbortSignal.timeout(15000) },
        );
        const text = await response.text();
        assert.ifError(model.failure);
        assert.equal(
          response.status,
          200,
          `${where}/${driver}/${scenario}: ${text}\n${logs}`,
        );
        const result = JSON.parse(text);
        if (["roundtrip", "retry", "non-streaming"].includes(scenario))
          assert.equal(result.text, "42 ✓");
        else assert.ok(result.error, `${scenario} must fail`);
        if (scenario === "unauthorized") {
          assert.equal(result.status, 401);
          assert.equal(result.kind, "authentication");
        }
        if (scenario === "too-large") assert.equal(result.status, 413);
        if (["roundtrip", "retry"].includes(scenario)) {
          assert.equal(result.input_tokens, 10);
          assert.equal(result.output_tokens, 4);
        }
        assert.ifError(model.failure);
        assert.equal(
          calls.length - before,
          scenario === "retry" ? 3 : scenario === "roundtrip" ? 2 : 1,
        );
        console.log(`ok ${where}/${driver}/${scenario}`);
      }
    }
  }
  const missing = await fetch(
    `http://127.0.0.1:${port}/worker?${new URLSearchParams({ base, driver: "missing", scenario: "non-streaming" })}`,
  ).then((r) => r.json());
  assert.match(missing.error, /injected HTTP transport/);
  assert.equal(leak, false, "redirect must never forward credentials");
  assert.equal(waiting.size, 0);
  console.log(
    `workerd provider integration passed (${calls.length} HTTP model requests)`,
  );
} finally {
  if (child && child.exitCode === null) {
    child.kill();
    await once(child, "exit");
  }
  model.closeAllConnections();
  await new Promise((resolve) => model.close(resolve));
  await rm(dir, { recursive: true, force: true });
}
