// Measure the complete deployment using the same pinned bundler as upload.
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const out = mkdtempSync(resolve(tmpdir(), "everruns-worker-size-"));
// Decimal MB is stricter than 3 MiB. Keep room for application additions by
// measuring every uploaded module, including JS glue and all Wasm modules.
const budget = Number(process.env.MAX_WORKER_GZIP_BYTES ?? 3_000_000);
if (!Number.isSafeInteger(budget) || budget <= 0) throw new Error("invalid size budget");
try {
  const run = spawnSync(process.execPath, [
    resolve(root, "crates/durable-object/worker/node_modules/wrangler/bin/wrangler.js"),
    "deploy", "--config", resolve(root, "examples/durable-object-agent/wrangler.jsonc"),
    "--dry-run", "--outdir", out,
  ], { cwd: root, encoding: "utf8", env: { ...process.env, WRANGLER_SEND_METRICS: "false" } });
  if (run.error) throw run.error;
  process.stdout.write(run.stdout);
  process.stderr.write(run.stderr);
  if (run.status !== 0) throw new Error(`Worker bundling failed (${run.status})`);
  const match = run.stdout.match(/Total Upload: ([\d.]+) KiB \/ gzip: ([\d.]+) KiB/);
  if (!match) throw new Error("Wrangler did not report complete upload size");
  // Wrangler rounds to .01 KiB. Use the upper bound rather than undercounting.
  const upper = Math.ceil((Number(match[2]) + 0.005) * 1024);
  console.log(`Compressed upload <= ${upper} bytes; budget ${budget}; headroom >= ${budget - upper} bytes.`);
  if (upper > budget) throw new Error("Engine Worker exceeds compressed upload budget");
} finally {
  rmSync(out, { recursive: true, force: true });
}
