#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const uiRoot = resolve(scriptDir, "..");
const repoRoot = resolve(uiRoot, "../..");
const specPath = resolve(repoRoot, "docs/api/openapi.json");
const outputPath = resolve(uiRoot, "src/lib/api/generated/openapi.ts");
const schemaTypesPath = resolve(uiRoot, "src/lib/api/schema-types.ts");
const legacyTypesPath = resolve(uiRoot, "src/lib/api/legacy-api-types.ts");
const check = process.argv.includes("--check");

const spec = JSON.parse(readFileSync(specPath, "utf8"));
const seenOperationIds = new Map();

for (const [path, item] of Object.entries(spec.paths ?? {})) {
  for (const method of ["get", "put", "post", "delete", "patch", "options", "head", "trace"]) {
    const operation = item?.[method];
    const operationId = operation?.operationId;
    if (!operationId) continue;

    const seen = seenOperationIds.get(operationId) ?? 0;
    seenOperationIds.set(operationId, seen + 1);
    if (seen > 0) {
      const suffix = `${method}_${path}`.replace(/[^A-Za-z0-9]+/g, "_").replace(/^_|_$/g, "");
      operation.operationId = `${operationId}_${suffix}`;
    }
  }
}

const tempDir = mkdtempSync(join(tmpdir(), "everruns-ui-openapi-"));
const tempSpecPath = join(tempDir, "openapi.json");
const tempOutputPath = join(tempDir, "openapi.ts");
const tempSchemaTypesPath = join(tempDir, "schema-types.ts");

function exportedLegacyTypeNames() {
  const source = readFileSync(legacyTypesPath, "utf8");
  const names = new Set(["EnqueueTaskRequest", "EnqueueTaskResponse"]);
  for (const match of source.matchAll(/^export\s+(?:interface|type|enum)\s+(\w+)/gm)) {
    names.add(match[1]);
  }
  return names;
}

function schemaTypesSource() {
  const legacyNames = exportedLegacyTypeNames();
  const schemaNames = Object.keys(spec.components?.schemas ?? {}).sort();
  const lines = [
    'import type { components } from "./generated/openapi";',
    "",
    'type Schemas = components["schemas"];',
    "",
    "export type ApiSchema<Name extends keyof Schemas> = Schemas[Name];",
    "",
  ];

  for (const name of schemaNames) {
    lines.push(`export type OpenApi${name} = Schemas[${JSON.stringify(name)}];`);
  }
  lines.push("");

  for (const name of schemaNames.filter((name) => !legacyNames.has(name))) {
    lines.push(`export type ${name} = Schemas[${JSON.stringify(name)}];`);
  }
  lines.push("");
  return lines.join("\n");
}

function formatFiles(paths) {
  execFileSync("pnpm", ["exec", "oxfmt", "--write", ...paths], { cwd: uiRoot, stdio: "inherit" });
}

try {
  writeFileSync(tempSpecPath, JSON.stringify(spec));
  const apiTypesOutputPath = check ? tempOutputPath : outputPath;
  const schemaTypesOutputPath = check ? tempSchemaTypesPath : schemaTypesPath;

  execFileSync(
    "pnpm",
    [
      "exec",
      "openapi-typescript",
      tempSpecPath,
      "--empty-objects-unknown",
      "-o",
      apiTypesOutputPath,
    ],
    { cwd: uiRoot, stdio: "inherit" },
  );
  writeFileSync(schemaTypesOutputPath, schemaTypesSource());
  formatFiles([apiTypesOutputPath, schemaTypesOutputPath]);

  if (check) {
    const current = readFileSync(outputPath, "utf8");
    const generated = readFileSync(tempOutputPath, "utf8");
    if (current !== generated) {
      console.error("Generated UI API types are stale. Run: pnpm run api-types:generate");
      process.exit(1);
    }
    const currentSchemas = readFileSync(schemaTypesPath, "utf8");
    const generatedSchemas = readFileSync(tempSchemaTypesPath, "utf8");
    if (currentSchemas !== generatedSchemas) {
      console.error("Generated UI schema aliases are stale. Run: pnpm run api-types:generate");
      process.exit(1);
    }
  }
} finally {
  rmSync(tempDir, { recursive: true, force: true });
}
