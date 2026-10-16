import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

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
      { cwd: root, encoding: "utf8" },
    ),
  );
  assert.equal(
    metadata.packages.find((p) => p.id === metadata.resolve.root).name,
    "everruns-cloudflare-durable-chat",
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

test("the deployed app binds only the native ChatRoom Durable Object", async () => {
  const raw = await readFile(join(root, "wrangler.jsonc"), "utf8");
  const config = JSON.parse(
    raw.replace(/^\s*\/\/.*$/gm, "").replace(/,\s*([}\]])/g, "$1"),
  );
  assert.deepEqual(config.durable_objects.bindings, [
    { name: "ROOMS", class_name: "ChatRoom" },
  ]);
  assert.equal(config.containers, undefined);
  assert.equal(config.services, undefined);
  const bundle = await readFile(join(root, "build/index.js"), "utf8");
  assert.match(bundle, /\bChatRoom\b/);
  assert.doesNotMatch(bundle, /celld|\bAgentCell\b/i);
});
