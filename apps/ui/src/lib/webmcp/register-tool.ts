import type { WebMcpToolDefinition } from "./types";

export function hasWebMcpSupport(doc: Document = document): boolean {
  return typeof doc.modelContext?.registerTool === "function";
}

export async function registerWebMcpTool(
  tool: WebMcpToolDefinition,
  signal: AbortSignal,
  doc: Document = document,
): Promise<boolean> {
  const modelContext = doc.modelContext;
  if (!modelContext) return false;
  await modelContext.registerTool(tool, { signal });
  return true;
}
