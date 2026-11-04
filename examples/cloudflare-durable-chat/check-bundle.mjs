// Check the complete pinned Wrangler upload, including JS glue and Wasm.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const out = mkdtempSync(join(tmpdir(), "everruns-chat-bundle-"));
try {
  const run = spawnSync(
    process.execPath,
    [
      join(root, "node_modules/wrangler/bin/wrangler.js"),
      "deploy",
      "--dry-run",
      "--outdir",
      out,
    ],
    {
      cwd: root,
      encoding: "utf8",
      env: { ...process.env, WRANGLER_SEND_METRICS: "false" },
    },
  );
  if (run.error) throw run.error;
  process.stdout.write(run.stdout);
  process.stderr.write(run.stderr);
  assert.equal(run.status, 0, "production Worker must bundle successfully");
  assert.equal(
    readdirSync(out, { recursive: true }).filter((p) => p.endsWith(".wasm"))
      .length,
    1,
    "authority commands must reuse the Engine Wasm",
  );
  const size = run.stdout.match(
    /Total Upload: ([\d.]+) KiB \/ gzip: ([\d.]+) KiB/,
  );
  assert.ok(size, "Wrangler must report the complete upload size");
  // Current platform limit is raw bytes, not compressed bytes. Wrangler rounds
  // to .01 KiB, so use the upper bound. Startup/CPU/memory need separate proof.
  const raw = Math.ceil((Number(size[1]) + 0.005) * 1024);
  assert.ok(
    raw <= 64 * 1024 * 1024,
    "Worker exceeds Cloudflare's raw size limit",
  );
  console.log(
    `One Engine Wasm; uncompressed upload <= ${raw} bytes (64 MiB platform limit).`,
  );
} finally {
  rmSync(out, { recursive: true, force: true });
}
