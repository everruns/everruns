import fs from "node:fs";
import path from "node:path";
import net from "node:net";
import http from "node:http";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";
// Explicit assets only: this script never installs, downloads, or selects a personal browser.
const required = (name) => {
  const value = process.env[name];
  assert(
    value && path.isAbsolute(value),
    `${name} must be an explicit absolute path`,
  );
  return value;
};
const source = required("BROWSE_E2E_SOURCE");
const binary = required("BROWSE_E2E_BINARY");
const chrome = required("BROWSE_E2E_CHROME");
const extension = required("BROWSE_E2E_EXTENSION");
const output = required("BROWSE_E2E_OUTPUT");
assert(
  !fs.existsSync(output),
  "Output directory must not already exist; use a fresh test directory",
);
const revision = spawnSync(
  "/usr/bin/git",
  ["-C", source, "rev-parse", "HEAD"],
  { encoding: "utf8" },
).stdout.trim();
assert.equal(
  revision,
  "c3b29be5299658281fe4ecdcb926bb7b5a9d6f8c",
  "Use the reviewed pinned upstream source",
);
const { BrowserCdp } = await import(
  pathToFileURL(path.join(source, "stress/lib/cdp.ts"))
);
const { McpClient } = await import(
  pathToFileURL(path.join(source, "stress/lib/mcp.ts"))
);
const { pairViaSettings } = await import(
  pathToFileURL(path.join(source, "stress/lib/installed.ts"))
);
fs.mkdirSync(output, { mode: 0o700 });
const root = output;
const home = path.join(root, "private-home");
const profile = path.join(root, "profile");
const evidence = [];
const children = [];
const clients = [];
let cdp, fixture;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const env = {
  PATH: "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin",
  HOME: home,
  TMPDIR: path.join(root, "tmp"),
  LANG: "en_US.UTF-8",
  OPENCODE_BROWSER_HOME: path.join(home, "browse"),
  OPENCODE_BROWSER_PORT: "19988",
  OPENCODE_BROWSER_TOOLS: "plain",
  OPENCODE_BROWSER_AUTOSTART: "0",
  OPENCODE_BROWSER_AUTO_UPDATE: "off",
  OPENCODE_BROWSER_UPDATE_CHECK: "0",
};
for (const p of [home, profile, env.TMPDIR])
  fs.mkdirSync(p, { recursive: true, mode: 0o700 });
// Imported upstream MCP helper merges process.env: remove all ambient credentials before spawning.
for (const k of Object.keys(process.env)) delete process.env[k];
Object.assign(process.env, env);

function start(file, args, name) {
  const fd = fs.openSync(path.join(root, name + ".log"), "w", 0o600);
  const child = spawn(file, args, {
    env,
    cwd: root,
    detached: true,
    stdio: ["ignore", fd, fd],
  });
  fs.closeSync(fd);
  children.push(child);
  return child;
}
async function until(fn, ms = 15000) {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    await sleep(150);
  }
  throw new Error("condition timeout: " + (last?.message ?? ""));
}
async function freePort(port) {
  await new Promise((res, rej) => {
    const s = net.createServer();
    s.once("error", rej);
    s.listen(port, "127.0.0.1", () => s.close(res));
  });
}
function record(name, detail) {
  evidence.push({ name, detail, at: new Date().toISOString() });
  fs.writeFileSync(
    path.join(root, "evidence.json"),
    JSON.stringify(evidence, null, 2),
  );
  console.log(name === "failed" ? "FAIL" : "PASS", name);
}
function value(r) {
  if (r.data) return r.data;
  try {
    return JSON.parse(r.text);
  } catch {
    return null;
  }
}
async function ok(c, name, args) {
  const r = await c.call(name, args);
  assert(!r.isError, `${name}: ${r.text.slice(0, 300)}`);
  return r;
}
let cleaning = false;
async function cleanup() {
  if (cleaning) return;
  cleaning = true;
  for (const c of clients) await c.close().catch(() => {});
  cdp?.close();
  fixture?.close();
  for (const child of children.reverse()) {
    if (child.exitCode !== null) continue;
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {}
    await sleep(500);
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {}
  }
  for (const dir of [home, profile, env.TMPDIR])
    fs.rmSync(dir, { recursive: true, force: true });
}
for (const sig of ["SIGINT", "SIGTERM"])
  process.on(sig, async () => {
    await cleanup();
    process.exit(130);
  });
try {
  for (const port of [
    19988, 19990, 19991, 19992, 19993, 19994, 19995, 19996, 19997,
  ])
    await freePort(port);
  assert(fs.existsSync(binary) && fs.existsSync(chrome), "Build assets first");
  start(binary, ["serve"], "relay");
  await until(() =>
    fs.existsSync(path.join(home, "browse/relays/19988/relay.json")),
  );
  record("owned relay ready");
  const pair = spawnSync(binary, ["pair", "--wait-seconds", "1"], {
    env,
    cwd: root,
    encoding: "utf8",
    timeout: 15000,
  });
  const code = /Pairing code: ([0-9A-Z]{4}-[0-9A-Z]{4})/.exec(pair.stdout)?.[1];
  assert(code, "pairing code absent");
  start(
    chrome,
    [
      "--disable-crash-reporter",
      "--disable-breakpad",
      `--user-data-dir=${profile}`,
      "--remote-debugging-address=127.0.0.1",
      "--remote-debugging-port=0",
      `--load-extension=${extension}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--password-store=basic",
      "about:blank",
    ],
    "chrome",
  );
  const port = await until(
    () =>
      fs.existsSync(path.join(profile, "DevToolsActivePort")) &&
      Number(
        fs
          .readFileSync(path.join(profile, "DevToolsActivePort"), "utf8")
          .split("\n")[0],
      ),
  );
  cdp = await BrowserCdp.connect(10000, port);
  await until(() => cdp.extensionWorker());
  record("production extension loaded");
  await pairViaSettings(cdp, code, "127.0.0.1:19988");
  record("real settings form submitted");
  const a = new McpClient([binary, "mcp"], env);
  clients.push(a);
  await a.initialize();
  await until(async () => {
    const r = await a.call("status", {}, 10000);
    return !r.isError && /"connected"\s*:\s*true/.test(r.text);
  }, 30000);
  record("real extension settings pairing");
  const discovery = await a.request("tools/list");
  assert(discovery.tools.some((t) => t.name === "navigate"));
  record("plain MCP tools discovery", discovery.tools.length);
  fixture = http.createServer((req, res) => {
    res.setHeader("Content-Type", "text/html");
    res.end(
      '<!doctype html><title>Disposable Browse E2E</title><h1>Synthetic local fixture</h1><label>Name<input id="name"></label><button id="save" onclick="document.querySelector(\"#result\").textContent=\"Saved \"+document.querySelector(\"#name\").value">Save</button><p id="result"></p>',
    );
  });
  await new Promise((r) => fixture.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${fixture.address().port}/`;
  record("synthetic consent only; not human approval");
  const nav = await ok(a, "navigate", { url });
  const tab = value(nav)?.tab;
  assert(Number.isInteger(tab), "navigate missing tab");
  record("navigate local fixture", { tab });
  const initial = await ok(a, "snapshot", { tab });
  const field = /[^\n]*Name[^\n]*\[ref=([^\]]+)\]/i.exec(initial.text)?.[1];
  const button = /[^\n]*Save[^\n]*\[ref=([^\]]+)\]/i.exec(initial.text)?.[1];
  assert(field && button, "fixture refs absent: " + initial.text);
  await ok(a, "fill", { tab, ref: field, value: "Synthetic test" });
  await ok(a, "click", { tab, ref: button });
  const snap = await ok(a, "snapshot", { tab });
  assert(snap.text.includes("Saved Synthetic test"));
  record("fill click snapshot");
  const shot = await ok(a, "screenshot", { tab });
  assert(shot.images > 0);
  record("screenshot image");
  const b = new McpClient([binary, "mcp"], env);
  clients.push(b);
  await b.initialize();
  assert((await b.call("snapshot", { tab })).isError);
  record("foreign MCP owner denied");
  const target = await until(async () =>
    (await cdp.targets()).find((t) => t.url === url),
  );
  let finished = false;
  const waiting = a.call("handoff", {
    tab,
    message: "AUTOMATED TEST ONLY: Click Continue. This is not human approval.",
    timeoutSeconds: 20,
  });
  const clicking = cdp.clickHandoffContinue(target.targetId, () => finished);
  const result = await waiting.finally(() => (finished = true));
  const clicks = await clicking;
  assert.equal(value(result)?.outcome, "continued");
  record("trusted test input completes real handoff", {
    clicks,
    syntheticConsent: true,
  });
  const untrusted = a.call("handoff", {
    tab,
    message: "AUTOMATED TEST ONLY: untrusted DOM click must not continue",
    timeoutSeconds: 3,
  });
  const attached = await cdp.send("Target.attachToTarget", {
    targetId: target.targetId,
    flatten: true,
  });
  try {
    const find = (n) =>
      n.nodeName === "BUTTON" &&
      n.attributes?.some(
        (x, i) => x === "id" && n.attributes[i + 1] === "continue",
      )
        ? n
        : [
            ...(n.children ?? []),
            ...(n.shadowRoots ?? []),
            ...(n.contentDocument ? [n.contentDocument] : []),
          ]
            .map(find)
            .find(Boolean);
    const button = await until(async () =>
      find(
        (
          await cdp.send(
            "DOM.getDocument",
            { depth: -1, pierce: true },
            attached.sessionId,
          )
        ).root,
      ),
    );
    const resolved = await cdp.send(
      "DOM.resolveNode",
      { backendNodeId: button.backendNodeId },
      attached.sessionId,
    );
    await cdp.send(
      "Runtime.callFunctionOn",
      {
        objectId: resolved.object.objectId,
        functionDeclaration: "function(){this.click()}",
      },
      attached.sessionId,
    );
    assert.equal(value(await untrusted)?.outcome, "timeout");
    record("untrusted DOM click ignored");
  } finally {
    await cdp.send("Target.detachFromTarget", {
      sessionId: attached.sessionId,
    });
  }
  const timeout = await a.call("handoff", {
    tab,
    message: "AUTOMATED TEST ONLY: waiting for timeout",
    timeoutSeconds: 1,
  });
  assert.equal(value(timeout)?.outcome, "timeout");
  record("handoff timeout");
  const closing = a.call("handoff", {
    tab,
    message: "AUTOMATED TEST ONLY: tab will close",
    timeoutSeconds: 15,
  });
  await sleep(500);
  await cdp.send("Target.closeTarget", { targetId: target.targetId });
  assert.equal(value(await closing)?.outcome, "closed");
  record("handoff tab closure");
} catch (error) {
  record("failed", { message: error.message });
  process.exitCode = 1;
} finally {
  await cleanup();
  record("cleanup completed");
}
