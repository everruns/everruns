import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFile, access } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { createHash } from "node:crypto";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

test("the resolved application graph contains no celld or sibling-example dependency", async () => {
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      [
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--offline",
        "--filter-platform",
        "wasm32-unknown-unknown",
      ],
      { cwd: root, encoding: "utf8", maxBuffer: 8 * 1024 * 1024 },
    ),
  );
  assert.equal(
    metadata.packages.find((p) => p.id === metadata.resolve.root).name,
    "everruns-cloudflare-durable-chat",
  );
  const adapter = metadata.packages.find(
    (p) => p.name === "everruns-durable-object",
  );
  assert.equal(adapter.publish.length, 0, "the hosting adapter is source-only");
  assert.ok(adapter.dependencies.some((d) => d.name === "everruns"));
  assert.ok(
    adapter.dependencies.every(
      (d) => !["everruns-core", "everruns-contracts"].includes(d.name),
    ),
    "the adapter must implement the public Framework SPI",
  );
  for (const pkg of metadata.packages) {
    assert.doesNotMatch(pkg.name, /celld/i);
    assert.doesNotMatch(
      pkg.manifest_path,
      /examples[/\\](?!cloudflare-durable-chat[/\\])/,
    );
  }
  const npm = JSON.parse(
    await readFile(join(root, "package-lock.json"), "utf8"),
  );
  for (const name of Object.keys(npm.packages))
    assert.doesNotMatch(name, /celld/i);
});

test("the app uses Framework hosting and only native Cloudflare objects", async () => {
  const raw = await readFile(join(root, "wrangler.jsonc"), "utf8");
  const config = JSON.parse(
    raw.replace(/^\s*\/\/.*$/gm, "").replace(/,\s*([}\]])/g, "$1"),
  );
  assert.deepEqual(config.durable_objects.bindings, [
    { name: "ROOMS", class_name: "ChatRoom" },
    { name: "SESSIONS", class_name: "ChatSession" },
  ]);
  assert.equal(config.containers, undefined);
  assert.equal(config.services, undefined);
  const app = await readFile(join(root, "src/lib.rs"), "utf8");
  assert.match(app, /Engine::builder/);
  assert.match(app, /ObjectHosting::new/);
  assert.match(app, /OpenRouterChatDriver::new/);
  assert.doesNotMatch(app, /ReasonAtom|ActAtom|plan_next_turn|struct Turn/);
  for (const file of ["src/engine.rs", "src/openai.rs", "src/worker.rs"])
    await assert.rejects(access(join(root, file)), { code: "ENOENT" });
  const worker = await readFile(join(root, "worker/index.js"), "utf8");
  assert.match(worker, /crates\/durable-object\/worker/);
  assert.doesNotMatch(worker, /celld|examples\//);
  await access(join(root, "pkg/everruns_cloudflare_durable_chat_bg.wasm"));
  assert.match(
    worker,
    /installSessionCommands\(session_command, session_command_key\)/,
  );
  assert.doesNotMatch(worker, /worker\/src\/index\.js/);
  const retained = JSON.parse(
    await readFile(join(root, "retained-builds.json"), "utf8"),
  );
  const policy = createHash("sha256")
    .update(await readFile(join(root, "src/agent.rs")))
    .digest("hex");
  assert.equal(
    policy,
    retained.policy_sha256,
    "retained registrations need their exact factory",
  );
  const build = JSON.parse(
    await readFile(join(root, "worker/build.json"), "utf8"),
  );
  assert.equal(build.build_id, `chat-${policy.slice(0, 24)}`);
});
