import specification from "../../../../docs/api/openapi.json";
import { docsPath } from "./site";

type DocsMetadata = { group: string; support: string; audience: string; availability: string };
export type ApiOperation = {
  id: string;
  path: string;
  method: string;
  title: string;
  tag: string;
  href: string;
  deprecated: boolean;
  docs: DocsMetadata;
  operation: Record<string, any>;
};
export type ApiGroup = { id: string; name: string; description: string; tags: string[] };
export const spec: Record<string, any> = specification;
const methods = new Set(["get", "post", "put", "patch", "delete", "head", "options", "trace"]);

// Fail the docs build when a new route has not been classified by the API owner.
export const groups: ApiGroup[] = (spec["x-tagGroups"] ?? []).map((g: any) => ({
  id: g["x-slug"], name: g.name, description: g["x-description"], tags: g.tags,
}));
if (groups[0]?.id !== "agent-api") throw new Error("OpenAPI must put the Agent API group first");
export const operations: ApiOperation[] = Object.entries(spec.paths).flatMap(([path, item]) =>
  Object.entries(item as Record<string, any>).filter(([method]) => methods.has(method)).map(([method, operation]) => {
    const docs = operation["x-docs"] as DocsMetadata;
    if (!docs || !groups.some(g => g.id === docs.group) || !["stable", "preview", "experimental"].includes(docs.support)
      || !["public", "internal"].includes(docs.audience)) throw new Error(`Missing API documentation classification: ${method} ${path}`);
    const id = operation.operationId;
    if (!/^[\w-]+$/.test(id)) throw new Error(`Unsafe operation ID: ${id}`);
    return {
      id, path, method: method.toUpperCase(), title: operation.summary || id.replaceAll("_", " "),
      tag: operation.tags[0], href: docsPath(`/api/operations/${id}/`),
      deprecated: operation.deprecated === true, docs, operation,
    };
  }),
);
export const byId = new Map(operations.map(o => [o.id, o]));
export const deprecatedGroup: ApiGroup = {
  id: "deprecated", name: "Deprecated", tags: [],
  description: "Historical aliases and archival APIs. Follow each replacement before building a new integration.",
};
export const allGroups = [...groups, deprecatedGroup];
export const groupHref = (id: string) => docsPath(`/api/groups/${id}/`);
export const groupOperations = (id: string) => operations.filter(o => id === "deprecated" ? o.deprecated : !o.deprecated && o.docs.group === id);
export const tagName = (name: string) => spec.tags.find((t: any) => t.name === name)?.["x-displayName"] ?? name;
export const operationTitle = (op: ApiOperation) => `${op.title.replace(/\.$/, "")}${op.deprecated ? " (legacy)" : ""}`;

export function apiPage(pathname: string) {
  const relative = pathname.slice(docsPath("/api/").length).replace(/\/$/, "");
  if (relative.startsWith("operations/") && !relative.startsWith("operations/tags/")) {
    return { operation: byId.get(relative.slice("operations/".length)) };
  }
  if (relative.startsWith("operations/tags/")) {
    const slug = relative.slice("operations/tags/".length);
    const tag = spec.tags.find((t: any) => t.name.toLowerCase().replaceAll(" ", "-") === slug)?.name;
    return { tag, group: groups.find(g => g.tags.includes(tag)) };
  }
  return { group: allGroups.find(g => g.id === relative.replace(/^groups\//, "")) ?? groups[0], overview: !relative };
}

export function exampleFor(schema: any, depth = 0, seen = new Set<string>()): any {
  if (!schema || depth > 6) return undefined;
  if (schema.example !== undefined) return schema.example;
  if (schema.examples?.length) return schema.examples[0];
  if (schema.default !== undefined) return schema.default;
  if (schema.enum?.length) return schema.enum.find((v: any) => v !== null) ?? null;
  if (schema.$ref) {
    if (seen.has(schema.$ref)) return undefined;
    return exampleFor(spec.components.schemas[schema.$ref.split("/").pop()], depth + 1, new Set([...seen, schema.$ref]));
  }
  const alternative = schema.oneOf ?? schema.anyOf;
  if (alternative) return exampleFor(alternative.find((s: any) => s.type !== "null"), depth + 1, seen);
  if (schema.allOf) return Object.assign({}, ...schema.allOf.map((s: any) => exampleFor(s, depth + 1, seen)));
  const type = Array.isArray(schema.type) ? schema.type.find((t: string) => t !== "null") : schema.type;
  if (schema.properties || type === "object") return Object.fromEntries(Object.entries(schema.properties ?? {}).flatMap(([key, value]) => {
    if (schema.required?.length && !schema.required.includes(key) && (value as any).example === undefined) return [];
    const result = exampleFor(value, depth + 1, seen);
    return result === undefined ? [] : [[key, result]];
  }));
  if (type === "array") return [exampleFor(schema.items, depth + 1, seen)].filter(v => v !== undefined);
  if (type === "boolean") return true;
  if (type === "integer" || type === "number") return schema.minimum ?? 0;
  if (type === "string") return schema.format === "date-time" ? "2026-10-12T09:00:00Z" : "string";
  return undefined;
}

export function responseExample(response: any, status?: string) {
  const content = response.content?.["application/json"] ?? response.content?.["application/problem+json"];
  if (!content) return undefined;
  if (content.example !== undefined) return content.example;
  const named = Object.values(content.examples ?? {}).find((v: any) => v.value !== undefined) as any;
  if (named) return named.value;
  if (Number(status) >= 400 && content.schema?.$ref?.endsWith("/ErrorResponse")) return { title: response.description, status: Number(status) };
  return exampleFor(content.schema);
}
