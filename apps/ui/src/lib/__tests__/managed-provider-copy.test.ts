/** @jest-environment node */

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

import { managedProviderCopy } from "@/lib/managed-provider-copy";

const SOURCE_ROOT = path.resolve(__dirname, "../..");
const COPY_MODULE = path.join(SOURCE_ROOT, "lib", "managed-provider-copy.ts");

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      return sourceFiles(entryPath);
    }
    return entry.name.endsWith(".tsx") || entry.name.endsWith(".ts") ? [entryPath] : [];
  });
}

test("managed-provider copy is non-empty", () => {
  for (const value of Object.values(managedProviderCopy)) {
    expect(value.trim().length).toBeGreaterThan(0);
  }
});

// A distribution overrides managed-provider-copy.ts to reword how it describes
// a provider it supplies. That only works while every surface reads the module,
// so no other file may spell the copy out inline.
test("no surface hardcodes the managed-provider copy", () => {
  // `badge` is a single common word; scanning for it would flag unrelated
  // prose. The two distinctive phrases are enough to catch a stray copy.
  const phrases = [managedProviderCopy.badgeTitle, managedProviderCopy.notice];

  const violations = sourceFiles(SOURCE_ROOT)
    .filter((filePath) => filePath !== COPY_MODULE && filePath !== __filename)
    .filter((filePath) => {
      const source = readFileSync(filePath, "utf8");
      // Collapse JSX line wrapping so a broken-up string still matches.
      const flattened = source.replace(/\s+/g, " ");
      return phrases.some((phrase) => flattened.includes(phrase));
    })
    .map((filePath) => path.relative(SOURCE_ROOT, filePath));

  expect(violations).toEqual([]);
});
