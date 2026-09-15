/** @jest-environment node */

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

const SOURCE_ROOT = path.resolve(__dirname, "../..");
const NESTED_LINK_BUTTON = /<Link\b[^>]*>\s*<Button\b/;

function tsxFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      return tsxFiles(entryPath);
    }
    return entry.name.endsWith(".tsx") ? [entryPath] : [];
  });
}

test("link-styled buttons do not nest interactive elements", () => {
  const violations = tsxFiles(SOURCE_ROOT)
    .filter((filePath) => NESTED_LINK_BUTTON.test(readFileSync(filePath, "utf8")))
    .map((filePath) => path.relative(SOURCE_ROOT, filePath));

  expect(violations).toEqual([]);
});
