// The reference is a public URL contract, including sitemap and Markdown discovery.
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { docsUrl } from "../site.config.mjs";

const app = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const spec = JSON.parse(readFileSync(path.join(app, "../../docs/api/openapi.json"), "utf8"));
const dist = path.join(app, "dist");
const sitemap = readFileSync(path.join(dist, "sitemap.xml"), "utf8");
const methods = new Set(["get", "post", "put", "patch", "delete", "head", "options", "trace"]);
const operations = Object.values(spec.paths).flatMap(item => Object.entries(item).filter(([method]) => methods.has(method)).map(([, op]) => op));
const page = (route) => {
  const file = path.join(dist, route, "index.html");
  assert(existsSync(file), `Missing API route: ${route}`);
  assert(existsSync(path.join(dist, route, "index.md")), `Missing API Markdown twin: ${route}`);
  assert(sitemap.includes(`<loc>${docsUrl(`/${route}/`)}</loc>`), `API route absent from sitemap: ${route}`);
  return readFileSync(file, "utf8");
};

assert.equal(spec["x-tagGroups"][0].name, "Agent API");
const overview = page("api");
assert(overview.includes("Call your agent."), "API overview must lead with the Agent API");
assert(overview.indexOf("Agent API</span>") < overview.indexOf("Agents &amp; authoring</span>"), "Agent API must be the first navigation group");
for (const group of spec["x-tagGroups"]) page(`api/groups/${group["x-slug"]}`);
page("api/groups/deprecated");

const ids = new Set(operations.map(op => op.operationId));
for (const operation of operations) {
  assert(operation["x-docs"], `Missing classification: ${operation.operationId}`);
  const html = page(`api/operations/${operation.operationId}`);
  assert(html.includes('data-api-reference'), `Operation uses the old renderer: ${operation.operationId}`);
  if (operation["x-superseded-by"]) {
    assert(ids.has(operation["x-superseded-by"]), `Unknown successor: ${operation.operationId}`);
    assert(html.includes(`/api/operations/${operation["x-superseded-by"]}/`), `Missing migration link: ${operation.operationId}`);
  }
}
const start = page("api/operations/agent_api_create_session");
assert(!start.includes('Caller-owned key/value'), "Platform session request must not advertise Framework-only metadata");
assert(start.includes("TypeScript") && start.includes("Python"), "Agent API SDK examples missing");
const index = JSON.parse(readFileSync(path.join(dist, "api/search.json"), "utf8"));
assert.equal(index.length, operations.length, "Endpoint search must cover every operation");
assert.equal(new Set(index.map(op => op.id)).size, operations.length, "Search IDs must be unique");
console.log(`API reference verified: ${operations.length} preserved operation URLs, ${spec["x-tagGroups"].length + 1} groups, sitemap, Markdown and endpoint search.`);
