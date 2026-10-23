// GHSA-mrhx-c7xc-m3w5: resolve the patched package while retaining the public
// v3 API for vetted consumers. Backport early collection bounds to legacy
// engines; never relabel versions or suppress advisories. Unknown bytes fail closed.
// THREAT[TM-DOS-049]: reject upper bounds before per-element allocation.
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const compatibility = JSON.parse(fs.readFileSync(new URL("./zod-v3-compatibility.json", import.meta.url), "utf8"));
const compatibilityBefore = new Map();
const compatibilityAfter = new Set();
const legacyConsumerMajors = new Set(Object.values(compatibility).filter((consumer) => Object.keys(consumer.files).length)
  .map((consumer) => `${consumer.package}@${consumer.version.split(".")[0]}`));
for (const consumer of Object.values(compatibility)) for (const file of Object.values(consumer.files)) {
  compatibilityBefore.set(file.before, file.after);
  compatibilityAfter.add(file.after);
}
export function compatibilityImports(source) {
  // Change only module specifiers. Preserve standard-schema vendor strings,
  // application data and the consumer's actual v3 API, including declarations.
  return source.replace(/(\bfrom\s*|\brequire\s*\(\s*|\bimport\s*\(\s*|\bimport\s*)(["'])zod\2/g,
    (_, prefix, quote) => prefix + quote + "zod/v3" + quote)
    .replace(/\bdefine\(\[([^\]]*)\]/g, (_, modules) => "define([" + modules.replace(/(["'])zod\1/g,
      (_, quote) => quote + "zod/v3" + quote) + "]");
}
export function isCompatibilitySource(source) {
  return compatibilityBefore.has(hash(source)) || compatibilityAfter.has(hash(source));
}
export const upstreamHashes = new Set([
  "9763a2938f0e7f983363ef4bf6530e7583e3f2d54f8a4a6684efad58febc6e8e", // 3.25.76 ESM
  "06487cc05ed45c5dd29789bf0aa75734d1ce06592a7f2665ba4ca90a68e8a3e1", // 3.25.76 CJS
  "d7214e1dacf9e6e70c9ca80247717b4be13b240510428fef132b7da3260d1a32", // 4.6.6–4.6.7/v3 ESM
  "7a2f06d88e280bdd198c60e0d292ed7f1e3993e877be4415de07e6abc7746dd1", // 4.6.6–4.6.7/v3 CJS
  "874a6a4dde402f563882621f01b4789a7a12c64bc8a7abb1e18731e97eb271f0", // design.md 0.4.0 bundled v3
  "a76ab9c2ee8a1bc6ad709167c3c7f06e64a5e2253d9718d3c2a8c51824272423", // design.md linter export
]);
const nextHash = "af7deaaff50a14c7d8b52f518e10dac800a350bf8eef43f8454006fa7a44e4da";
upstreamHashes.add(nextHash);
const legacyV4Hashes = new Set([
  "d466582c022b08a3f9eb1a65d5970d8ca25064d73a5a838d76414ae36440a76b", // 3.25.76/v4 ESM
  "f5d17a7db487c943c9d07cc624010ebb6d93683eabd14339846f4912cc88b51f", // 3.25.76/v4 CJS
  "17dc355f16ef7f76f3fd01399932280b1b5accc3dc83e9e8823e7f985c190be1", // license-verifier 0.5.0 bundled 4.3.6 CJS
  "1493aad710c85598540c34abc02a5e383a7b1ca491da83b387498df32fd92dfe", // license-verifier ESM
  "2d1ccc01d21b7d54ca99a6305991d7fb6e072ea4e725d7fec5eb6184188c52c9", // Playwright 1.63.0 protocol bundle
]);
// Next compiles its private v3 copy into a single CJS module. Use its own
// exported parser primitives; do not rewrite minified code or swap engines.
const nextGuard = `
(() => {
    const z = module.exports;
    for (const [Class, type] of [[z.ZodArray, "array"], [z.ZodSet, "set"]]) {
        const parse = Class.prototype._parse;
        Class.prototype._parse = function(input) {
            const value = input.data;
            const validType = type === "array" ? Array.isArray(value) : value instanceof Set;
            if (validType) {
                const size = type === "array" ? value.length : value.size;
                const exact = type === "array" && this._def.exactLength;
                const max = type === "array" ? this._def.maxLength : this._def.maxSize;
                const bound = exact && size > exact.value ? exact : max;
                const maximum = bound?.value;
                if (maximum >= 0 && size > maximum) {
                    z.addIssueToContext(this._getOrReturnCtx(input), {
                        code: "too_big", maximum, type, inclusive: true,
                        exact: bound === exact, message: bound.message
                    });
                    return z.INVALID;
                }
            }
            return parse.call(this, input);
        };
    }
})();
`;
// Same leading-check rule as upstream util.sizeCaps/capSize, executed before
// parse/runChecks so older engines cannot duplicate the rejecting issue.
const v4Guard = `
            // everruns: legacy-v4 collection bound before walking elements
            if (!ctx?.skipChecks && ctx?.direction !== "backward") {
                const value = payload.value;
                const type = inst._zod.def.type;
                const size = type === "array" && Array.isArray(value) ? value.length
                    : type === "map" && value instanceof Map || type === "set" && value instanceof Set ? value.size : -1;
                if (size >= 0) for (const check of inst._zod.def.checks ?? []) {
                    const bound = check._zod.def;
                    if (!/_(length|size)$|_equals$/.test(bound.check)) break;
                    const maximum = bound.maximum ?? bound.length ?? bound.size;
                    if (maximum >= 0 && size > maximum && (!bound.when || bound.when(payload))) {
                        const start = payload.issues.length;
                        check._zod.check(payload);
                        if (payload.issues.length > start) {
                            payload.issues[start].continue = false;
                            return payload;
                        }
                    }
                }
            }
`;
const marker = "\n/* everruns: zod-v3-bound-before-walk */\n";
export const hash = (text) => createHash("sha256").update(text).digest("hex");

export function backport(original) {
  if (hash(original) === nextHash) return original + nextGuard;
  if (legacyV4Hashes.has(hash(original))) {
    let count = 0;
    const patched = original.replace(/inst\._zod\.run = \(payload, ctx\) => \{/g, (header) => {
      count++;
      return header + v4Guard;
    });
    if (count !== 1) throw new Error("Missing legacy v4 execution boundary");
    return patched;
  }
  // Each replacement is confined to one upstream class and one existing bound.
  // Preserve lower-bound behavior, messages, valid inputs and sync/async API.
  function patchClass(source, name, bounds) {
    const start = source.indexOf(`class ${name} extends ZodType {`);
    const end = source.indexOf(`${name}.create =`, start);
    if (start < 0 || end < 0) throw new Error(`Missing ${name} implementation`);
    let body = source.slice(start, end);
    const invalid = body.match(/return ([\w.]*INVALID);/)?.[1];
    if (!invalid) throw new Error(`Missing ${name} abort value`);
    for (const [field, exact] of bounds) {
      const pattern = new RegExp(`(if \\(def\\.${field} !== null\\) \\{[\\s\\S]*?)status\\.dirty\\(\\);`);
      let count = 0;
      body = body.replace(pattern, (_, prefix) => {
        count++;
        return prefix + (exact ? `if (tooBig) return ${invalid};\n                status.dirty();` : `return ${invalid};`);
      });
      if (count !== 1) throw new Error(`Missing ${name}.${field} bound`);
    }
    return source.slice(0, start) + body + source.slice(end);
  }
  let source = patchClass(original, "ZodArray", [["exactLength", true], ["maxLength", false]]);
  source = patchClass(source, "ZodSet", [["maxSize", false]]);
  return source;
}

export function upstreamSource(source) {
  if (source.endsWith(marker)) {
    // A marker alone is never evidence: reconstruct and authenticate preimage.
    const patched = source.slice(0, -marker.length);
    const original = patched.endsWith(nextGuard) ? patched.slice(0, -nextGuard.length)
      : patched.includes(v4Guard) ? patched.replace(v4Guard, "") : patched
      .replace(/if \(tooBig\) return ([\w.]*INVALID);\n                status\.dirty\(\);/, "status.dirty();")
      .replace(/(if \(def\.maxLength !== null\) \{[\s\S]*?)return ([\w.]*INVALID);/, "$1status.dirty();")
      .replace(/(if \(def\.maxSize !== null\) \{[\s\S]*?)return ([\w.]*INVALID);/, "$1status.dirty();");
    if (!(upstreamHashes.has(hash(original)) || legacyV4Hashes.has(hash(original))) || backport(original) !== patched) throw new Error("Unverified patched Zod source");
    return original;
  }
  if (!(upstreamHashes.has(hash(source)) || legacyV4Hashes.has(hash(source)))) throw new Error(`Unknown Zod v3 source: ${hash(source)}`);
  return source;
}

export function patchedSource(source) {
  if (compatibilityAfter.has(hash(source))) return source;
  const expected = compatibilityBefore.get(hash(source));
  if (expected) {
    const patched = compatibilityImports(source);
    if (hash(patched) !== expected) throw new Error("Unexpected v3 compatibility transformation");
    return patched;
  }
  return backport(upstreamSource(source)) + marker;
}

export function installationRoot(directory = process.cwd()) {
  directory = fs.realpathSync(directory);
  // The OSS UI can also be installed inside the SaaS npm workspace. Its hoisted
  // dependencies belong to that checkout, not to a separate external location.
  for (let parent = path.dirname(directory); parent !== path.dirname(parent); parent = path.dirname(parent)) {
    const manifest = path.join(parent, "package.json");
    if (!fs.existsSync(manifest)) continue;
    const pkg = JSON.parse(fs.readFileSync(manifest, "utf8"));
    if (Array.isArray(pkg.workspaces) && pkg.workspaces.includes(path.relative(parent, directory))) return parent;
  }
  return directory;
}

export function findSources(root = installationRoot()) {
  const sources = new Set();
  const seen = new Set();
  root = fs.realpathSync(root);
  function inside(file) {
    const real = fs.realpathSync(file);
    if (!real.startsWith(root + path.sep)) throw new Error("Refusing dependency outside checkout");
    return real;
  }
  function modules(directory) {
    if (!fs.existsSync(directory)) return;
    directory = inside(directory);
    if (seen.has(directory)) return;
    seen.add(directory);
    for (const entry of fs.readdirSync(directory)) {
      const file = path.join(directory, entry);
      if (entry === ".pnpm") {
        for (const pkg of fs.readdirSync(file)) modules(path.join(file, pkg, "node_modules"));
      } else if (entry.startsWith("@")) {
        for (const pkg of fs.readdirSync(file)) visit(path.join(file, pkg));
      } else if (!entry.startsWith(".")) visit(file);
    }
  }
  function visit(directory) {
    const manifest = path.join(directory, "package.json");
    if (!fs.existsSync(manifest)) return;
    const pkg = JSON.parse(fs.readFileSync(manifest, "utf8"));
    const consumer = compatibility[`${pkg.name}@${pkg.version}`];
    const legacyRange = /^[~^]?3\./.test(pkg.dependencies?.zod ?? "") && !/\|\|\s*[~^>=]*4(?:\.|\b)/.test(pkg.dependencies.zod);
    const knownLegacyMajor = legacyConsumerMajors.has(`${pkg.name}@${String(pkg.version ?? "").split(".")[0]}`);
    if ((legacyRange || knownLegacyMajor) && !consumer) throw new Error(`Review new v3 consumer: ${pkg.name}@${pkg.version}`);
    if (consumer) for (const relative of Object.keys(consumer.files)) sources.add(inside(path.join(directory, relative)));
    if (pkg.name === "zod") {
      if (!["3.25.76", "4.6.6", "4.6.7"].includes(pkg.version)) throw new Error(`Review new Zod version: ${pkg.version}`);
      for (const name of ["types.js", "types.cjs"]) {
        const file = path.join(directory, "v3", name);
        if (fs.existsSync(file)) sources.add(inside(file));
      }
      if (pkg.version === "3.25.76") {
        for (const name of ["schemas.js", "schemas.cjs"]) sources.add(inside(path.join(directory, "v4/core", name)));
      }
    } else if (pkg.name === "@google/design.md") {
      for (const name of ["dist/index.js", "dist/linter/index.js"]) sources.add(inside(path.join(directory, name)));
    }
    else if (pkg.name === "@copilotkit/license-verifier") {
      for (const name of ["index.js", "index.mjs"]) sources.add(inside(path.join(directory, "dist", name)));
    }
    else if (pkg.name === "next") sources.add(inside(path.join(directory, "dist/compiled/zod/index.cjs")));
    else if (pkg.name === "playwright-core") sources.add(inside(path.join(directory, "lib/utilsBundle.js")));
    modules(path.join(directory, "node_modules"));
  }
  modules(path.join(root, "node_modules"));
  const manifest = path.join(root, "package.json");
  const workspaces = JSON.parse(fs.readFileSync(manifest, "utf8")).workspaces ?? [];
  for (const workspace of workspaces) modules(path.join(root, workspace, "node_modules"));
  return [...sources].sort();
}

export function patchInstalled({ check = false, root = installationRoot() } = {}) {
  const files = findSources(root);
  if (files.length === 0) throw new Error("No installed Zod v3 sources found");
  // Validate the entire set before writing any file. Replace inodes, never mutate
  // a pnpm content-store hardlink or a symlink target outside this checkout.
  const plan = files.map((file) => {
    const before = fs.readFileSync(file, "utf8");
    const after = patchedSource(before);
    if (check && before !== after) throw new Error(`Missing Zod backport: ${path.relative(root, file)}`);
    return { file, before, after };
  });
  for (const { file, before, after } of plan) {
    if (before === after) continue;
    const temporary = `${file}.security-${process.pid}`;
    fs.writeFileSync(temporary, after, { flag: "wx", mode: fs.statSync(file).mode });
    fs.renameSync(temporary, file);
  }
  return { verified: plan.length, changed: plan.filter(({ before, after }) => before !== after).length };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  console.log(JSON.stringify(patchInstalled({ check: process.argv.includes("--check") })));
}
