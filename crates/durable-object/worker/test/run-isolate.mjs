// Actual workerd/Wasm crash recovery. Uses public test credentials and synthetic input.
import { spawn } from "node:child_process";
import { mkdtempSync, openSync, closeSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "node:net";
import assert from "node:assert/strict";
const here = dirname(fileURLToPath(import.meta.url));
const run = mkdtempSync(join(tmpdir(), "everruns-wasm-recovery."));
const probe = createServer();
await new Promise((r) => probe.listen(0, "127.0.0.1", r));
const port = probe.address().port;
await new Promise((r) => probe.close(r));
const fd = openSync(join(run, "workerd.log"), "w");
const child = spawn(
  process.execPath,
  [
    join(here, "../node_modules/wrangler/bin/wrangler.js"),
    "dev",
    "--minify",
    "--config",
    join(here, "qualification.json"),
    "--ip",
    "127.0.0.1",
    "--port",
    String(port),
    "--inspector-port",
    "0",
    "--persist-to",
    join(run, "state"),
    "--var",
    "AUTHORITY_TOKEN:authority-test-token-at-least-32-bytes",
  ],
  {
    detached: true,
    env: { ...process.env, WRANGLER_SEND_METRICS: "false" },
    stdio: ["ignore", fd, fd],
  },
);
closeSync(fd);
try {
  let ready = false;
  for (let i = 0; i < 150; i++) {
    assert.equal(child.exitCode, null, `workerd exited; logs: ${run}`);
    try {
      await fetch(`http://127.0.0.1:${port}/`, { signal: AbortSignal.timeout(500) });
      ready = true;
      break;
    } catch {}
    await new Promise((r) => setTimeout(r, 200));
  }
  assert.ok(ready, `workerd readiness timeout; logs: ${run}`);
  const check = spawn(process.execPath, [join(here, "qualify-isolate.mjs")], {
    env: { ...process.env, DO_QUALIFY_URL: `http://127.0.0.1:${port}` },
    stdio: "inherit",
  });
  const code = await new Promise((r, j) => {
    check.once("error", j);
    check.once("exit", r);
  });
  assert.equal(code, 0);
  console.log(`Wasm recovery evidence: ${run}`);
} finally {
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {}
  await new Promise((r) => setTimeout(r, 500));
  try {
    process.kill(-child.pid, "SIGKILL");
  } catch {}
}
