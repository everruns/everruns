import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import os from "node:os";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import { findSources, patchInstalled, patchedSource, upstreamSource, isCompatibilitySource, compatibilityImports } from "./patch-zod-v3.mjs";

async function verifyBounds(z) {
  let visited = 0;
  const string = z.string ?? (() => z.ZodString.create());
  const arrayOf = z.array ?? ((element) => z.ZodArray.create(element));
  const element = string().refine(() => { visited++; return true; });
  const array = arrayOf(element);
  const cases = [[array.max(2, "bounded"), ["a", "b", "c"]], [array.length(2), ["a", "b", "c"]]];
  if (z.set || z.ZodSet) {
    const set = z.set ? z.set(element) : z.ZodSet.create(element);
    cases.push([set.max(2), new Set(["a", "b", "c"])], [set.size(2), new Set(["a", "b", "c"])]);
  }
  if (z.map) {
    const map = z.map(element, element);
    const bounded = typeof map.max === "function" ? map.max(2) : z.maxSize && map.check(z.maxSize(2));
    if (bounded) cases.push([bounded, new Map([["a", "b"], ["c", "d"], ["e", "f"]])]);
  }
  for (const [schema, oversized] of cases) {
    for (const async of [false, true]) {
      visited = 0;
      const result = async ? await schema.safeParseAsync(oversized) : schema.safeParse(oversized);
      assert.equal(result.success, false);
      assert.equal(result.error.issues.length, 1);
      assert.equal(result.error.issues[0].code, "too_big");
      assert.equal(visited, 0, "an upper bound must reject before visiting elements");
    }
  }
  visited = 0;
  assert.equal(array.max(2).parse(["a", "b"]).join(","), "a,b");
  assert.equal(visited, 2);
  visited = 0;
  assert.equal(array.min(2).safeParse(["a"]).error.issues[0].code, "too_small");
  assert.equal(visited, 1, "lower bounds preserve v3 parsing behavior");
  assert.equal(array.max(2, "bounded").safeParse([1, 2, 3]).error.issues[0].message, "bounded");
  const refined = array.refine(() => true);
  if (typeof refined.max === "function") {
    visited = 0;
    assert.equal(refined.max(2).safeParse(["a", "b", "c"]).success, false);
    assert.equal(visited, 3, "v4 preserves a custom collection check before its later size bound");
  }
  visited = 0;
  const asyncArray = arrayOf(string().refine(async () => { visited++; return true; })).max(2);
  assert.equal((await asyncArray.safeParseAsync(["a", "b", "c"])).success, false);
  assert.equal(visited, 0);
  assert.equal((await asyncArray.safeParseAsync(["a", "b"])).success, true);
  assert.equal(visited, 2);
}

function bundledEngine(source) {
  const start = source.search(/\/\/ [^\n]*zod@3\.25\.76\/node_modules\/zod\/v3\/external\.js\n/);
  const end = source.indexOf("// src/linter/parser/spec.ts", start);
  assert.ok(start >= 0 && end > start);
  // Execute only the vendored schema engine, never the CLI or its file/network actions.
  const context = vm.createContext({ __export: (target, getters) => {
    for (const [key, get] of Object.entries(getters)) Object.defineProperty(target, key, { get });
  }, Set, Map });
  return vm.runInContext(source.slice(start, end) + ";({ZodArray,ZodSet,ZodString})", context, { timeout: 1000 });
}

function bundledV4(source) {
  const start = source.indexOf("//#region ../../node_modules/.pnpm/zod@4.3.6/node_modules/zod/v4/core/core.js");
  const end = source.indexOf("//#region src/", start);
  assert.ok(start >= 0 && end > start);
  const context = vm.createContext({ Set, Map });
  return vm.runInContext(source.slice(start, end) + ";({array,string})", context, { timeout: 1000 });
}

test("unknown or forged installed sources fail closed", () => {
  assert.throws(() => patchedSource("not upstream source"), /Unknown Zod/);
  assert.throws(() => patchedSource("forged\n/* everruns: zod-v3-bound-before-walk */\n"), /Unverified patched/);
});

test("compatibility redirects only module imports and preserves application strings", () => {
  const input = `import { z } from "zod"; const z = require('zod'); type T = import("zod").ZodType; const vendor = "zod"; define(["react","zod"], factory);`;
  const result = compatibilityImports(input);
  assert.equal(result, `import { z } from "zod/v3"; const z = require('zod/v3'); type T = import("zod/v3").ZodType; const vendor = "zod"; define(["react","zod/v3"], factory);`);
});

test("install validates the full set before writes and preserves a shared-store hardlink", () => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "everruns-zod-install-"));
  try {
    const root = path.join(fixture, "checkout");
    const directory = path.join(root, "node_modules/zod/v3");
    fs.mkdirSync(directory, { recursive: true });
    fs.writeFileSync(path.join(root, "package.json"), '{"name":"fixture"}');
    fs.writeFileSync(path.dirname(directory) + "/package.json", '{"name":"zod","version":"4.6.7"}');
    const packagePath = path.dirname(createRequire(import.meta.url).resolve("zod/package.json"));
    const source = upstreamSource(fs.readFileSync(packagePath + "/v3/types.js", "utf8"));
    const cjs = upstreamSource(fs.readFileSync(packagePath + "/v3/types.cjs", "utf8"));
    const store = path.join(fixture, "store.js");
    fs.writeFileSync(store, source);
    fs.linkSync(store, directory + "/types.js");
    fs.writeFileSync(directory + "/types.cjs", "tampered");
    assert.throws(() => patchInstalled({ root }), /Unknown/);
    assert.equal(fs.readFileSync(directory + "/types.js", "utf8"), source);
    fs.writeFileSync(directory + "/types.cjs", cjs);
    assert.equal(patchInstalled({ root }).changed, 2);
    assert.equal(fs.readFileSync(store, "utf8"), source, "shared content-store inode must remain untouched");
    assert.equal(patchInstalled({ root, check: true }).changed, 0);
    fs.rmSync(directory + "/types.js");
    fs.symlinkSync(store, directory + "/types.js");
    assert.throws(() => patchInstalled({ root }), /outside checkout/);
    assert.equal(fs.readFileSync(store, "utf8"), source);
  } finally { fs.rmSync(fixture, { recursive: true, force: true }); }
});

test("resolved packages are patched v4 while the legacy AG-UI schema API remains v3", async () => {
  if (fs.existsSync("package-lock.json")) {
    const lock = JSON.parse(fs.readFileSync("package-lock.json", "utf8"));
    for (const [name, pkg] of Object.entries(lock.packages)) {
      if (name.endsWith("node_modules/zod")) assert.equal(pkg.version, "4.6.7");
    }
  } else {
    const lock = fs.readFileSync("pnpm-lock.yaml", "utf8");
    assert.doesNotMatch(lock, /^  zod@3\./m);
  }
  const file = findSources().find((file) => file.endsWith("/@ag-ui/core/dist/index.mjs"));
  assert.ok(file, "the legacy dependent must be exercised, not just the direct Zod package");
  const core = await import(pathToFileURL(file).href);
  const z3 = await import("zod/v3");
  const schema = core.RunAgentInputSchema;
  assert.ok(schema instanceof z3.ZodType);
  assert.equal(schema._def.typeName, "ZodObject");
  const input = { threadId: "fixture", runId: "fixture", state: {}, messages: [], tools: [], context: [], forwardedProps: {} };
  assert.equal(schema.safeParse(input).success, true);
  assert.equal(schema.safeParse({ ...input, threadId: 7 }).success, false);
  // CopilotKit uses this converter in SaaS. It is absent from the OSS UI tree.
  let converter;
  try { converter = createRequire(file)("zod-to-json-schema"); }
  catch (error) { if (error.code !== "MODULE_NOT_FOUND") throw error; }
  if (converter) assert.equal(converter.zodToJsonSchema(schema).properties.threadId.type, "string");
});

test("every installed legacy engine is patched, including ESM, CJS and bundled tooling", async () => {
  const files = findSources();
  assert.ok(files.length > 0);
  for (const file of files) {
    const source = fs.readFileSync(file, "utf8");
    const patched = patchedSource(source);
    assert.equal(patchedSource(patched), patched, "patch must be idempotent and authenticated");
    assert.throws(() => patchedSource(patched + "\n/* tampered */"), /Unverified|Unknown/);
    if (isCompatibilitySource(source)) continue;
    if (file.includes("/license-verifier/")) {
      const before = bundledV4(upstreamSource(source));
      let visited = 0;
      before.array(before.string().refine(() => { visited++; return true; })).max(2).safeParse(["a", "b", "c"]);
      assert.equal(visited, 3);
      await verifyBounds(bundledV4(patched));
    } else if (file.includes("/v4/core/")) {
      const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "everruns-zod-v4-"));
      try {
        fs.cpSync(path.dirname(path.dirname(file)), fixture + "/v4", { recursive: true });
        const schemaPath = path.join(fixture, "v4/core", path.basename(file));
        fs.writeFileSync(schemaPath, upstreamSource(source));
        const extension = file.endsWith(".cjs") ? ".cjs" : ".js";
        const before = await import(pathToFileURL(path.join(fixture, "v4/index" + extension)).href);
        let visited = 0;
        before.array(before.string().refine(() => { visited++; return true; })).max(2).safeParse(["a", "b", "c"]);
        assert.equal(visited, 3);
        // A separate tree gives both module formats an independent cache key.
        fs.cpSync(fixture + "/v4", fixture + "/patched", { recursive: true });
        fs.writeFileSync(path.join(fixture, "patched/core", path.basename(file)), patched);
        await verifyBounds(await import(pathToFileURL(path.join(fixture, "patched/index" + extension)).href));
      } finally { fs.rmSync(fixture, { recursive: true, force: true }); }
    } else if (/\/@google\/design\.md\/dist\/(?:linter\/)?index\.js$/.test(file)) {
      assert.equal(patched.startsWith("#!"), source.startsWith("#!"), "CLI shebang preserved");
      await verifyBounds(bundledEngine(patched));
      const z = bundledEngine(upstreamSource(source));
      let visited = 0;
      z.ZodArray.create(z.ZodString.create().refine(() => { visited++; return true; })).max(2).safeParse(["a", "b", "c"]);
      assert.equal(visited, 3, "published bundled v3 reproduces the original unnecessary element walk");
    } else {
      const isPlaywright = file.includes("/playwright-core/");
      const isNext = file.includes("/next/");
      const extension = file.endsWith(".cjs") || isPlaywright ? ".cjs" : ".js";
      const fixture = path.join(path.dirname(file), `everruns-bound-fixture-${process.pid}${extension}`);
      try {
        fs.writeFileSync(fixture, upstreamSource(source), { flag: "wx" });
        const loaded = await import(pathToFileURL(fixture).href + "?before");
        const before = isNext || isPlaywright ? loaded.default : loaded;
        let visited = 0;
        const string = isPlaywright ? before.CallToolRequestSchema.shape.params.shape.name : before.ZodString.create();
        const element = string.refine(() => { visited++; return true; });
        const array = isPlaywright ? element.array() : before.ZodArray.create(element);
        array.max(2).safeParse(["a", "b", "c"]);
        assert.equal(visited, 3, "published v3 reproduces the original unnecessary element walk");
        fs.rmSync(fixture);
        fs.writeFileSync(fixture, patched, { flag: "wx" });
        // CJS module caching uses the physical filename, unlike ESM query keys.
        const patchedFixture = fixture.replace("everruns-bound-fixture", "everruns-patched-fixture");
        fs.renameSync(fixture, patchedFixture);
        try {
          const result = await import(pathToFileURL(patchedFixture).href);
          const z = isNext ? result.default : isPlaywright ? {
            string: () => result.default.CallToolRequestSchema.shape.params.shape.name,
            array: (element) => element.array(),
          } : result;
          await verifyBounds(z);
        }
        finally { fs.rmSync(patchedFixture, { force: true }); }
      } finally {
        fs.rmSync(fixture, { force: true });
      }
    }
  }
  const result = patchInstalled({ check: true });
  assert.equal(result.verified, files.length);
  assert.equal(result.changed, 0);
});
