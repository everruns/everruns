import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
execFileSync(process.execPath, [resolve(root, "scripts/generate-agent-schema.mjs"), "--check"], { stdio: "inherit" });
const target = resolve(root, "apps/docs/public/schemas/agent/v1.json");
mkdirSync(dirname(target), { recursive: true });
copyFileSync(resolve(root, "docs/schemas/agent/v1.json"), target);
