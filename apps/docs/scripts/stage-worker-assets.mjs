// Lays out the build for the docs Worker: the static assets binding serves
// files by their public path, and the docs live under /docs, so dist/ is
// copied to dist-worker/docs/. The asset server reads _redirects only from the
// assets root, so it moves up a level (its paths are already /docs-prefixed).
//
// Astro writes each `redirects` entry in astro.config.mjs as an HTML page with
// a meta refresh. Search engines treat that as a soft signal, so every such
// page also becomes a real 301 in _redirects. See wrangler.jsonc.
import { appendFileSync, cpSync, existsSync, readFileSync, readdirSync, renameSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { BASE } from "../site.config.mjs";

const appRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(appRoot, "dist");
const out = path.join(appRoot, "dist-worker");
const docs = path.join(out, BASE.replace(/^\/+/, ""));

if (!existsSync(path.join(dist, "index.html"))) {
  throw new Error("dist/index.html missing: run `pnpm run build` first");
}
rmSync(out, { recursive: true, force: true });
cpSync(dist, docs, { recursive: true });
const redirects = path.join(out, "_redirects");
if (existsSync(path.join(docs, "_redirects"))) renameSync(path.join(docs, "_redirects"), redirects);

const REFRESH = /<meta http-equiv="refresh" content="0;url=([^"]+)"/;
const pages = (directory) =>
  readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    return entry.isDirectory() ? pages(file) : entry.name === "index.html" ? [file] : [];
  });
const rules = [];
for (const page of pages(docs)) {
  const target = readFileSync(page, "utf8").match(REFRESH)?.[1];
  if (!target) continue;
  const from = `/${path.relative(out, path.dirname(page)).split(path.sep).join("/")}`;
  rules.push(`${from}/ ${target} 301`, `${from} ${target} 301`);
}
appendFileSync(redirects, `\n# Generated from astro.config.mjs redirects\n${rules.join("\n")}\n`);

console.log(
  `Staged ${path.relative(appRoot, dist)}/ as ${path.relative(appRoot, docs)}/ for the docs Worker, with ${rules.length / 2} redirect(s).`,
);
