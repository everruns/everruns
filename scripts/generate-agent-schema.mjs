// Publish the portable authoring schema from the same types as the API reference.
// Semantic checks involving asset bytes or destination bindings stay in the codec.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const schemas = JSON.parse(readFileSync(resolve(root, "docs/api/openapi.json"), "utf8")).components.schemas;
const defs = {};
function convert(value) {
  if (Array.isArray(value)) return value.map(convert);
  if (!value || typeof value !== "object") return value;
  const result = {};
  for (const [key, child] of Object.entries(value)) {
    if (key === "$ref") {
      const name = child.replace("#/components/schemas/", "");
      if (!Object.hasOwn(defs, name)) {
        if (!schemas[name]) throw new Error(`Missing schema ${name}`);
        defs[name] = {};
        defs[name] = convert(schemas[name]);
      }
      result[key] = `#/definitions/${name}`;
    } else result[key] = convert(child);
  }
  return result;
}
defs.Manifest = convert(schemas.Manifest);
const manifest = defs.Manifest;
manifest.required = ["name"];
manifest.properties.schema_version = { ...manifest.properties.schema_version, enum: [1], default: 1 };
manifest.properties.name = { ...manifest.properties.name, pattern: "^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$", minLength: 1, maxLength: 255 };
manifest.properties.max_iterations = { ...manifest.properties.max_iterations, minimum: 1, maximum: 1000 };
manifest.properties.files.maxItems = 100;
manifest.properties.channels.maxProperties = 32;
// Packages accept client-side declarations only; builtin tools come from capabilities.
defs.ToolDefinition = convert(schemas.ClientSideTool);
defs.ToolDefinition.properties.type = { type: "string", enum: ["client_side"] };
defs.ToolDefinition.required.push("type");
defs.ToolDefinition.additionalProperties = false;
manifest.properties.starters.items = convert({ $ref: "#/components/schemas/ConversationStarter" });
manifest.allOf = [{ not: { required: ["instructions_file", "instructions"], properties: { instructions: { minLength: 1 }, instructions_file: { type: "string" } } } }];
for (const name of ["Manifest", "FileSource", "InitialFile", "PackageModel", "PackageCapabilityReference", "Channel", "NetworkAccessList", "ScopedMcpServer"]) {
  if (defs[name]) defs[name].additionalProperties = false;
}
defs.InitialFile.properties.encoding = { ...defs.InitialFile.properties.encoding, enum: ["text", "base64"], default: "text" };
defs.InitialFile.properties.is_readonly.default = true;
defs.InitialFile.properties.path.description = "Relative destination in the session file tree; leading / and /workspace/ are accepted aliases.";
defs.FileSource.properties.is_readonly.default = true;
defs.PackageCapabilityReference.properties.config.default = {};
defs.Channel.properties.enabled.default = false;
defs.Channel.properties.config.default = {};
defs.Channel.properties.config.type = "object";
defs.PackageCapabilityReference.properties.config.type = "object";
if (!defs.ScopedMcpServer) throw new Error("Manifest must expose scoped MCP server settings");
// serde defaults and try_from are not represented by the shared MCP OpenAPI type.
delete defs.ScopedMcpServer.required;
const document = {
  $schema: "http://json-schema.org/draft-04/schema#",
  id: "https://docs.everruns.com/schemas/agent/v1.json",
  title: "Everruns agent.toml v1",
  description: "Canonical agent.toml authoring schema. Also applies to parsed YAML and JSON. Validate asset paths, skills, credentials and destination dependencies with everruns agents validate.",
  $ref: "#/definitions/Manifest",
  definitions: Object.fromEntries(Object.entries(defs).sort(([a], [b]) => a.localeCompare(b))),
};
const destination = resolve(root, "docs/schemas/agent/v1.json");
const content = JSON.stringify(document, null, 2) + "\n";
if (process.argv.includes("--check")) {
  if (readFileSync(destination, "utf8") !== content) throw new Error("Agent schema is stale; run node scripts/generate-agent-schema.mjs after exporting OpenAPI.");
} else {
  mkdirSync(dirname(destination), { recursive: true });
  writeFileSync(destination, content);
}
