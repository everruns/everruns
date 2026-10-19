import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

// Utoipa stores vendor extensions in randomized maps. Canonicalize only those
// keys at the export boundary, preserving standard field and authored array order.
export function canonicalize(value, extension = false) {
  if (Array.isArray(value)) return value.map(item => canonicalize(item, extension));
  if (value === null || typeof value !== "object") return value;
  const keys = Object.keys(value);
  const ordered = extension ? keys.sort() : [
    ...keys.filter(key => !key.startsWith("x-")),
    ...keys.filter(key => key.startsWith("x-")).sort(),
  ];
  return Object.fromEntries(ordered.map(key => [key, canonicalize(value[key], extension || key.startsWith("x-"))]));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const file = process.argv[2];
  writeFileSync(file, JSON.stringify(canonicalize(JSON.parse(readFileSync(file, "utf8"))), null, 2) + "\n");
}
