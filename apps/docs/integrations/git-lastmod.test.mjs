import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { newestDate, pageSources, parseGitLog } from "./git-lastmod.mjs";

test("parseGitLog keeps each path's newest date", () => {
  const dates = parseGitLog("@2026-11-10\ndocs/a.md\n\n@2026-10-01\ndocs/a.md\ndocs/b.md\n");
  assert.equal(dates.get("docs/a.md"), "2026-11-10");
  assert.equal(dates.get("docs/b.md"), "2026-10-01");
});

test("pageSources resolves pages, leaf bundles and API operations", () => {
  const root = mkdtempSync(join(tmpdir(), "docs-lastmod-"));
  const docs = join(root, "docs");
  const file = (path) => {
    mkdirSync(join(docs, path, ".."), { recursive: true });
    writeFileSync(join(docs, path), "x");
    return join(docs, path);
  };
  try {
    const home = file("index.mdx");
    const quickstart = file("framework/quickstart.md");
    const frameworkIndex = file("framework/index.md");
    const entry = file("changelog/2026-11-01-x/index.md");
    const shot = file("changelog/2026-11-01-x/shot@2x.png");
    const spec = file("api/openapi.json");

    assert.deepEqual(pageSources(docs, ""), [home]);
    assert.deepEqual(pageSources(docs, "framework/quickstart"), [quickstart]);
    // framework/ holds other pages, so it is not a bundle: only its index counts.
    assert.deepEqual(pageSources(docs, "framework"), [frameworkIndex]);
    assert.deepEqual(pageSources(docs, "changelog/2026-11-01-x").sort(), [entry, shot].sort());
    assert.deepEqual(pageSources(docs, "api/operations/list_agents"), [spec]);
    assert.deepEqual(pageSources(docs, "missing/page"), []);
    // Changelog listings depend on every entry.
    assert.deepEqual(pageSources(docs, "changelog/framework").sort(), [entry, shot].sort());
    assert.deepEqual(pageSources(docs, "changelog").sort(), [entry, shot].sort());

    const dates = new Map([
      ["docs/changelog/2026-11-01-x/index.md", "2026-11-01"],
      ["docs/changelog/2026-11-01-x/shot@2x.png", "2026-11-03"],
    ]);
    assert.equal(newestDate([entry, shot], dates, root), "2026-11-03");
    assert.equal(newestDate([], dates, root), undefined);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
