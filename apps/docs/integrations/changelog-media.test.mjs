import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import changelogMedia from "./changelog-media.mjs";

test("publishes a video with its native 2x poster alongside the entry", () => {
  const root = mkdtempSync(path.join(tmpdir(), "changelog-media-"));
  try {
    const docs = path.join(root, "docs");
    const entry = path.join(docs, "changelog", "demo");
    const output = path.join(root, "output");
    mkdirSync(entry, { recursive: true });
    for (const name of ["walkthrough.mp4", "workspace-poster@2x.png", "legacy-poster.png", "other.png"])
      writeFileSync(path.join(entry, name), name);
    changelogMedia({ docsDir: docs }).hooks["astro:build:done"]({ dir: pathToFileURL(output + "/") });
    for (const name of ["walkthrough.mp4", "workspace-poster@2x.png", "legacy-poster.png"])
      assert.equal(readFileSync(path.join(output, "changelog", "demo", name), "utf8"), name);
    assert.equal(existsSync(path.join(output, "changelog", "demo", "other.png")), false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
