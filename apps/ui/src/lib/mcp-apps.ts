// MCP Apps (SEP-1865) host helpers for the console: read the view a tool call
// recorded, build the frame's content security policy, and talk to the server
// endpoints that load views and relay a view's calls.
//
// See knowledge/integrations/mcp-apps-host.md.

import { api } from "@/lib/api/client";
import type { ContentPart } from "@/lib/api/types";

/** What `tool.completed.details.mcp_app` records about a call's view. */
export interface McpAppCall {
  server: string;
  tool: string;
  resource_uri: string;
  structured_content?: unknown;
  meta?: unknown;
}

export type McpInteractiveViews = "on" | "console_only" | "off";

export interface McpAppViewPolicy {
  connect_domains: string[];
  resource_domains: string[];
  frame_domains: string[];
  prefers_border?: boolean;
}

export interface McpAppView {
  html: string;
  policy: McpAppViewPolicy;
  interactive_views: McpInteractiveViews;
  can_call_tools: boolean;
}

export type McpAppToolCallResponse =
  | { status: "done"; result: Record<string, unknown> }
  | { status: "confirmation_required"; title: string; description?: string };

const VIEW_URI_MAX = 512;

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The view a completed tool call carries, if any (and only a `ui://` one). */
export function readMcpAppCall(details: unknown): McpAppCall | null {
  if (!isPlainObject(details)) return null;
  const app = details.mcp_app;
  if (!isPlainObject(app)) return null;
  const { server, tool, resource_uri: uri } = app;
  if (typeof server !== "string" || typeof tool !== "string" || typeof uri !== "string") {
    return null;
  }
  if (!uri.startsWith("ui://") || uri.length <= 5 || uri.length > VIEW_URI_MAX) return null;
  return {
    server,
    tool,
    resource_uri: uri,
    structured_content: app.structured_content,
    meta: app.meta,
  };
}

const ORIGIN_RE = /^(https|wss):\/\/(\*\.)?[a-z0-9.-]+\.[a-z0-9-]+(:\d+)?$/i;

function origins(list: string[] | undefined): string {
  return (list ?? []).filter((origin) => ORIGIN_RE.test(origin)).join(" ");
}

/**
 * The CSP the frame enforces. Scripts and styles may be inline (views are
 * single-file bundles); everything that reaches the network is limited to the
 * origins the view declared, and nothing else. The server already dropped
 * anything that is not an https/wss origin; this filters again so a bad value
 * can never widen the policy.
 */
export function buildViewCsp(policy: McpAppViewPolicy | undefined): string {
  const resources = origins(policy?.resource_domains);
  const connect = origins(policy?.connect_domains);
  const frames = origins(policy?.frame_domains);
  const directives = [
    "default-src 'none'",
    `script-src 'unsafe-inline' 'unsafe-eval' blob: ${resources}`.trim(),
    `style-src 'unsafe-inline' ${resources}`.trim(),
    `img-src data: blob: ${resources}`.trim(),
    `font-src data: ${resources}`.trim(),
    `media-src data: blob: ${resources}`.trim(),
    connect ? `connect-src ${connect}` : "connect-src 'none'",
    frames ? `frame-src ${frames}` : "frame-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
  ];
  return directives.join("; ");
}

/** Put the CSP first in the document's head, before any of the view's own tags. */
export function withCsp(html: string, csp: string): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${csp.replace(/"/g, "&quot;")}">`;
  const head = /<head[^>]*>/i.exec(html);
  if (head) {
    const at = head.index + head[0].length;
    return html.slice(0, at) + meta + html.slice(at);
  }
  return `<!DOCTYPE html><html><head>${meta}</head><body>${html}</body></html>`;
}

/**
 * The MCP `CallToolResult` for a completed call, rebuilt from what the event
 * stores: the text the model saw plus the view-only structured data.
 */
export function callToolResultFor(
  result: ContentPart[] | undefined,
  error: string | undefined,
  app: McpAppCall,
): Record<string, unknown> {
  const content: Array<Record<string, unknown>> = [];
  if (error) content.push({ type: "text", text: error });
  for (const part of result ?? []) {
    if (part.type === "text") content.push({ type: "text", text: part.text });
  }
  const out: Record<string, unknown> = { content, isError: Boolean(error) };
  if (app.structured_content !== undefined) out.structuredContent = app.structured_content;
  if (app.meta !== undefined) out._meta = app.meta;
  return out;
}

export async function getMcpAppView(sessionId: string, app: McpAppCall): Promise<McpAppView> {
  const params = new URLSearchParams({ server: app.server, uri: app.resource_uri });
  const response = await api.get<McpAppView>(
    `/v1/sessions/${sessionId}/mcp-apps/view?${params.toString()}`,
  );
  return response.data;
}

export async function callMcpAppTool(
  sessionId: string,
  request: { server: string; name: string; arguments: unknown; confirmed: boolean },
): Promise<McpAppToolCallResponse> {
  const response = await api.post<McpAppToolCallResponse>(
    `/v1/sessions/${sessionId}/mcp-apps/tools-call`,
    request,
  );
  return response.data;
}

export async function readMcpAppResource(
  sessionId: string,
  request: { server: string; uri: string },
): Promise<Record<string, unknown>> {
  const response = await api.post<Record<string, unknown>>(
    `/v1/sessions/${sessionId}/mcp-apps/resources-read`,
    request,
  );
  return response.data;
}

/** Window event a view's `ui/message` raises; the session's composer fills its draft. */
export const COMPOSE_DRAFT_EVENT = "everruns:compose-draft";

export interface ComposeDraftDetail {
  sessionId: string;
  text: string;
}
