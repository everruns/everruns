// The official runner is unmodified. Keep every raw check, and distinguish the
// supported discovery requirements from the core listen-stream prerequisite.
import { spawn } from "node:child_process";
import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
const [dir, base, binary, receiverFile] = process.argv.slice(2);
const output = path.resolve(".local/mcp-events-conformance/results");
await mkdir(output, { recursive: true });
async function checksIn(root) {
  const files = await readdir(root, { withFileTypes: true });
  for (const file of files) {
    const name = path.join(root, file.name);
    if (file.isDirectory()) {
      const checks = await checksIn(name);
      if (checks) return checks;
    } else if (file.name === "checks.json")
      return JSON.parse(await readFile(name, "utf8"));
  }
}
const summary = [];
for (const [role, scenario] of [
  ["server", "events-discovery"],
  ["client", "events-client-no-extension"],
  ["client", "events-client-webhook"],
  ["server", "events-webhook-delivery"],
]) {
  const resultDir = path.join(output, `${scenario}-${Date.now()}`);
  const delivery = scenario === "events-webhook-delivery";
  const args = [
    ...(delivery
      ? [
          "--import",
          path.resolve("scripts/mcp-events-conformance/receiver-port.mjs"),
        ]
      : []),
    path.join(dir, "dist/index.js"),
    role,
    "--scenario",
    scenario,
    "--spec-version",
    "2026-07-28",
    "--force",
    "--output-dir",
    resultDir,
  ];
  if (delivery) args.push("--timeout", "90000");
  args.push(
    ...(role === "server"
      ? ["--url", `${base}/mcp`]
      : ["--command", "exec bash scripts/mcp-events-conformance/client.sh"]),
  );
  const child = spawn(process.execPath, args, {
    env: {
      ...process.env,
      EVERRUNS_EVENTS_TEST_BINARY: binary,
      ...(delivery
        ? {
            EVERRUNS_EVENTS_RECEIVER_FILE: receiverFile,
            EVENTS_WEBHOOK_CALLBACK_BASE: "https://8.8.8.8",
            EVENTS_RECEIVER_WELL_KNOWN: "0",
            EVENTS_DELIVERY_WAIT_MS: "3000",
            EVENTS_DELIVERY_SETTLE_MS: "3500",
          }
        : {}),
    },
    stdio: "inherit",
  });
  const code = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", resolve);
  });
  const checks = await checksIn(resultDir);
  if (!checks?.length)
    throw new Error(`${scenario} emitted no checks (exit ${code})`);
  // Everruns exposes no core subscriptions/listen stream. listChanged is false,
  // and its notification rows are skipped. This prerequisite is reported by
  // upstream as a failure/untestable; it is never counted as a passed scenario.
  const outOfScope = new Set(
    role === "server" ? ["sep-3415-capability-list-changed-gated"] : [],
  );
  const untestablePrerequisite = (c) =>
    outOfScope.has(c.id) &&
    c.details?.untestable === true &&
    c.errorMessage?.includes("subscriptions/listen") &&
    c.errorMessage?.includes("-32601");
  const failures = checks.filter(
    (c) => c.status === "FAILURE" && !untestablePrerequisite(c),
  );
  const passed = checks.filter((c) => c.status === "SUCCESS");
  const required = {
    "events-discovery": [
      "sep-3415-capability-events-object",
      "sep-3415-list-implemented",
      "sep-3415-descriptor-delivery-subset",
      "sep-3415-error-not-found",
      "sep-3415-mcp-name-list-exempt",
    ],
    "events-client-no-extension": [
      "sep-3415-client-no-events-without-extension",
    ],
    "events-webhook-delivery": [
      "sep-3415-delivery-signature-formula",
      "sep-3415-delivery-retry-regenerates-signature",
      "sep-3415-delivery-standard-webhooks-headers",
      "sep-3415-delivery-410-non-retryable",
      "sep-3415-delivery-413-non-retryable",
      "sep-3415-verification-challenge-echo",
      "sep-3415-ssrf-reject-non-routable",
    ],
    "events-client-webhook": [
      "sep-3415-client-refresh-before-expiry",
      "sep-3415-client-null-cursor-not-replayed",
      "sep-3415-client-mcp-name-set",
    ],
  }[scenario];
  const missing = required.filter((id) => !passed.some((c) => c.id === id));
  if (missing.length)
    throw new Error(`${scenario}: required checks did not pass: ${missing}`);
  if (!passed.length || failures.length)
    throw new Error(`${scenario}: ${JSON.stringify(failures)}`);
  const row = {
    scenario,
    runnerExitCode: code,
    passed: passed.map((c) => c.id),
    unverified: checks.filter((c) => c.status !== "SUCCESS"),
    fullScenarioPassed: code === 0,
  };
  summary.push(row);
  console.log(JSON.stringify(row));
}
await writeFile(
  path.join(output, "summary.json"),
  JSON.stringify(summary, null, 2),
);
