import { createRequire } from "node:module";
import { resolve } from "node:path";
import { readFileSync, writeFileSync } from "node:fs";
const require = createRequire(resolve("node_modules/@everruns/generative-ui/package.json"));
const { standardUI, exportA2UICatalog } = await import(
  require.resolve("@copilotkit/react-core/v2/generative-ui")
);
const { GENERATIVE_UI_MARKDOWN_GUIDELINES } = await import(require.resolve("@copilotkit/shared"));
const catalog = exportA2UICatalog(standardUI);
function typeOf(schema) {
  if (schema.const !== undefined) return JSON.stringify(schema.const);
  if (schema.enum) return schema.enum.map((value) => JSON.stringify(value)).join(" | ");
  if (schema.$ref) return schema.$ref.split("/").at(-1);
  const union = schema.anyOf ?? schema.oneOf;
  if (union)
    return (
      union
        .filter((item) => !/DataBinding|FunctionCall/.test(item.$ref ?? ""))
        .map(typeOf)
        .join(" | ") || "DynamicValue"
    );
  if (schema.type === "array") return `(${typeOf(schema.items ?? {})})[]`;
  if (schema.type === "object")
    return `{${Object.entries(schema.properties ?? {})
      .map(
        ([name, prop]) => `${name}${schema.required?.includes(name) ? "" : "?"}: ${typeOf(prop)}`,
      )
      .join(", ")}}`;
  return Array.isArray(schema.type) ? schema.type.join(" | ") : (schema.type ?? "unknown");
}
const frequency = new Map();
for (const component of Object.values(catalog.components))
  for (const name of Object.keys(component.properties))
    frequency.set(name, (frequency.get(name) ?? 0) + 1);
const common = new Set(
  [...frequency]
    .filter(([name, count]) => name !== "component" && count > 35)
    .map(([name]) => name),
);
const describe = (schema, omit = new Set()) =>
  Object.entries(schema.properties ?? {})
    .filter(([name]) => name !== "component" && !omit.has(name))
    .map(([name, prop]) => `${name}${schema.required?.includes(name) ? "" : "?"}: ${typeOf(prop)}`)
    .join(", ");
const signatures = Object.entries(catalog.components)
  .map(([name, schema]) => `${name}(${describe(schema, common)}) — ${schema.description ?? ""}`)
  .join("\n");
const functions = Object.entries(catalog.functions)
  .map(
    ([name, schema]) =>
      `${name}(${describe(schema.properties.args ?? {})}) — ${schema.description ?? ""}`,
  )
  .join("\n");
const commonProps = [...common]
  .map((name) => {
    const schema = Object.values(catalog.components).find((c) => c.properties[name]).properties[
      name
    ];
    return `${name}?: ${typeOf(schema)}`;
  })
  .join(", ");
const output = `# A2UI 1.0 standard catalog

Use this catalog in regular chat and Work Chat. Emit A2UI 1.0 operations in a fenced a2ui block, never HTML or JavaScript. Supported surface operations are createSurface, updateComponents, updateDataModel and deleteSurface. Every operation includes version: "v1.0". Use catalogId: "${standardUI.id}".

${GENERATIVE_UI_MARKDOWN_GUIDELINES}

## Components (69)

A component is {id, component, ...props}; props are flat, not inside a props object. All props may be literals, {"@path":"/path"}, or {"@call":"function", "args":{...}}. Child references are component ids. The root id is "root". Optional shared presentation props on components that support them: ${commonProps}.

${signatures}

## Browser functions

${functions}

Only use inline surfaces here; no named page targets are advertised by these chat hosts. User actions send chat messages through the existing authorization and approval policies. Never emit callRendererFunction or callAgentFunction operations. Do not use Query or Mutation to bypass those policies.
`;
const target = resolve("../../crates/core/src/builtins/a2ui/standard-catalog.md");
if (process.argv.includes("--check")) {
  if (readFileSync(target, "utf8") !== output)
    throw new Error(
      "A2UI catalog is stale: run node generative-ui/scripts/export-catalog.mjs from apps/ui",
    );
} else writeFileSync(target, output);
console.log(
  `A2UI catalog ${standardUI.id}: ${Object.keys(catalog.components ?? {}).length} components, ${output.length} bytes`,
);
