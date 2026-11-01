import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

const sourceRoot = join(process.cwd(), "src");
const rawPaletteUtility =
  /(?<![\w-])(?:[\w-]+:)*[\w-]+-(?:red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-(?:50|100|200|300|400|500|600|700|800|900|950)(?!\d)(?:\/\d+)?/g;

const rawPaletteExceptions: Record<string, Record<string, number>> = {
  "app/(main)/durable/workers/page.tsx": {
    "bg-green-500": 1,
    "bg-red-500": 1,
    "bg-yellow-500": 1,
    "text-blue-600": 1,
    "text-green-600": 1,
    "text-orange-600": 1,
    "text-yellow-600": 1,
  },
  "app/(main)/durable/workflows/[workflowId]/page.tsx": {
    "text-blue-500": 1,
    "text-purple-500": 1,
  },
  "app/(main)/evals/[evalId]/page.tsx": {
    "text-green-600": 1,
    "text-red-600": 1,
    "text-yellow-600": 1,
  },
  "app/(main)/evals/[evalId]/runs/[runId]/page.tsx": {
    "text-green-600": 1,
    "text-red-600": 1,
    "text-yellow-600": 1,
  },
  "app/(main)/evals/page.tsx": {
    "text-green-600": 1,
    "text-red-600": 1,
    "text-yellow-600": 1,
  },
  "app/dev/file-tree/page.tsx": {
    "dark:text-amber-400": 1,
    "dark:text-sky-400": 1,
    "dark:text-violet-400": 1,
    "text-amber-500": 1,
    "text-amber-600": 1,
    "text-sky-600": 1,
    "text-violet-600": 1,
  },
  "app/dev/model-picker/page.tsx": {
    "fill-yellow-400": 1,
    "text-yellow-400": 1,
  },
  "components/ai-elements/file-tree.tsx": {
    "text-amber-500": 1,
    "text-amber-500/80": 1,
  },
  "components/files/file-browser.tsx": {
    "dark:text-amber-400": 1,
    "dark:text-sky-400": 1,
    "dark:text-violet-400": 1,
    "text-amber-600": 1,
    "text-sky-600": 1,
    "text-violet-600": 1,
  },
  "components/initial-files-editor.tsx": {
    "dark:text-amber-400": 1,
    "dark:text-emerald-400": 1,
    "dark:text-sky-400": 1,
    "dark:text-violet-400": 1,
    "text-amber-600": 1,
    "text-emerald-600": 1,
    "text-sky-600": 1,
    "text-violet-600": 1,
  },
  "components/models/model-picker.tsx": {
    "fill-yellow-400": 1,
    "hover:text-yellow-400": 1,
    "text-yellow-400": 1,
  },
  "components/observers/quality-summary.tsx": {
    "text-green-600": 1,
    "text-red-600": 1,
    "text-yellow-600": 1,
  },
  "components/session/cost/context-card.tsx": {
    "bg-amber-500": 1,
    "bg-cyan-600": 1,
    "bg-emerald-600": 1,
    "bg-orange-500": 1,
    "bg-pink-500": 1,
    "bg-sky-500": 1,
    "bg-violet-500": 1,
  },
};

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      return entry.name === "__tests__" ? [] : sourceFiles(path);
    }
    return /\.(?:css|ts|tsx)$/.test(entry.name) ? [path] : [];
  });
}

describe("semantic status colors", () => {
  it("keeps raw palette utilities limited to audited non-status encodings", () => {
    const actual: Record<string, Record<string, number>> = {};

    for (const path of sourceFiles(sourceRoot)) {
      const matches = readFileSync(path, "utf8").match(rawPaletteUtility);
      if (!matches) continue;

      const file = relative(sourceRoot, path);
      actual[file] = {};
      for (const utility of matches) {
        actual[file][utility] = (actual[file][utility] ?? 0) + 1;
      }
    }

    expect(actual).toEqual(rawPaletteExceptions);
  });
});
