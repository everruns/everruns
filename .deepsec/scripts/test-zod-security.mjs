import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import vm from "node:vm";
import test from "node:test";

const require = createRequire(import.meta.url);
const cli = require.resolve("deepsec");
const deepsecRequire = createRequire(cli);
const sandboxRoot = dirname(deepsecRequire.resolve("@vercel/sandbox/package.json"));
const sandboxRequire = createRequire(join(sandboxRoot, "package.json"));
const zodRoot = dirname(sandboxRequire.resolve("zod/package.json"));

test("MCP rate limiter uses patched IPv6 validation", () => {
  const mcpRequire = createRequire(require.resolve("@modelcontextprotocol/sdk/server/index.js"));
  const rateLimitRequire = createRequire(mcpRequire.resolve("express-rate-limit"));
  const { Address6 } = rateLimitRequire("ip-address");
  assert.equal(rateLimitRequire("ip-address/package.json").version, "10.7.4");
  assert.equal(Address6.isValid("fe80::1%eth0"), true);
  assert.equal(Address6.isValid("%".repeat(1000) + "\n"), false);
  assert.equal(Address6.isValid("invalid"), false);
});

async function verifyBounds(z) {
  let visits = 0;
  const item = z.string().superRefine(() => visits++);
  for (const [schema, input] of [
    [z.array(item).max(2, "bounded"), ["a", "b", "c"]],
    [z.array(item).length(2, "bounded"), ["a", "b", "c"]],
    [z.set(item).max(2, "bounded"), new Set(["a", "b", "c"])],
  ]) {
    visits = 0;
    const result = schema.safeParse(input);
    assert.equal(result.success, false);
    assert.equal(result.error.issues.length, 1);
    assert.equal(result.error.issues[0].message, "bounded");
    assert.equal(visits, 0);
    assert.equal((await schema.safeParseAsync(input)).success, false);
    assert.equal(visits, 0);
  }
  assert.equal(JSON.stringify(z.array(z.string()).max(2).parse(["a"])), '["a"]');
  assert.equal(z.array(item).min(2).safeParse(["a"]).success, false);
  assert.equal(z.array(item).length(2).safeParse(["a"]).success, false);
  assert.equal(
    z
      .set(item)
      .min(2)
      .safeParse(new Set(["a"])).success,
    false,
  );
  assert.equal(z.array(item).max(2).safeParse([42]).success, false);
  assert.equal((await z.array(z.string()).max(2).safeParseAsync(["a"])).success, true);
}

for (const entry of ["zod/v3", "zod/v3/index.cjs"]) {
  test(`patched public compatibility engine: ${entry}`, async () => {
    const z = entry.endsWith(".cjs")
      ? require(join(zodRoot, "v3/index.cjs")).z
      : (await import(pathToFileURL(join(zodRoot, "v3/index.js")).href)).z;
    assert.equal(sandboxRequire("zod/package.json").version, "4.6.7");
    await verifyBounds(z);
  });
}

function embeddedEngine(source) {
  const start = source.indexOf(
    "// ../../node_modules/.pnpm/zod@3.25.76/node_modules/zod/v3/external.js",
  );
  const end = source.indexOf("var NEVER = INVALID;", start);
  assert.ok(start >= 0 && end > start, "review changed Deepsec bundle layout");
  // Evaluate only the self-contained validation library, never CLI/scanner/provider code.
  return vm.runInNewContext(
    "const __export = (target, all) => { for (const key in all) Object.defineProperty(target, key, {get: all[key]}); };\n" +
      source.slice(start, end) +
      "\nexternal_exports;",
    { Set, Map, Date, Promise },
    { timeout: 1000 },
  );
}

for (const file of ["config.mjs", "cli.mjs"]) {
  test(`embedded ${file}: original traverses 3, patched rejects before traversal`, async () => {
    const source = readFileSync(join(dirname(cli), file), "utf8");
    const original = source
      .replace("        if (tooBig) return INVALID;\n", "")
      .replace(
        /(message: def\.maxLength\.message\n\s*\}\);\n\s*)return INVALID;/,
        "$1status.dirty();",
      )
      .replace(
        /(message: def\.maxSize\.message\n\s*\}\);\n\s*)return INVALID;/,
        "$1status.dirty();",
      );
    const before = embeddedEngine(original);
    let visits = 0;
    before
      .array(before.string().superRefine(() => visits++))
      .max(2)
      .safeParse(["a", "b", "c"]);
    assert.equal(visits, 3);
    const after = embeddedEngine(source);
    await verifyBounds(after);
  });
}

test("real Sandbox v3 record schemas remain compatible without creating a sandbox", async () => {
  for (const ext of ["js", "cjs"]) {
    const url = pathToFileURL(join(sandboxRoot, "dist/api-client/validators." + ext));
    const { InjectionRuleValidator } = await import(url.href);
    const input = { domain: "example.invalid", headers: { "X-Test": "fixture" } };
    assert.deepEqual(InjectionRuleValidator.parse(input), input);
    assert.equal(InjectionRuleValidator.safeParse({ domain: 42 }).success, false);
  }
});
