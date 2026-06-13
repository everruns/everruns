/**
 * Shared utilities for tool-call card components
 */

import type { ContentPart } from "@/lib/api/types";
import { isToolCallPart, isToolResultPart } from "@/lib/api/types";

/**
 * Common tool call content structure
 */
export interface ToolCallContent {
  id: string;
  name: string;
  /** Human-readable display name for UI rendering */
  display_name?: string;
  arguments: Record<string, unknown>;
}

/**
 * Common tool result content structure
 */
export interface ToolResultContent {
  tool_call_id: string;
  result?: unknown;
  error?: string;
}

export interface McpAppResource {
  uri: string;
  mimeType: string;
  html: string;
}

const MCP_CONTENT_WRAPPER_PARSE_LIMIT = 512 * 1024;
const MCP_CARD_HTML_MAX_BYTES = 64 * 1024;
// Validates ui://everruns/{entity}/{id}/card — entity is kebab-case, id starts
// with a lowercase alphanumeric char. Rejects uppercase, leading separators,
// single-char IDs, and forged authorities.
const MCP_CARD_URI_RE = /^ui:\/\/everruns\/[a-z][a-z-]*\/[a-z0-9][a-z0-9_-]+\/card$/;
// Accepts first-party card tools (no mcp_ prefix, e.g. agent_get_card) or
// Everruns' own MCP server tools (mcp_everruns__<entity>_get_card).
// External servers that happen to name a tool *_get_card are rejected.
const MCP_CARD_TOOL_RE = /^(?:[a-z]+_get_card|mcp_everruns__[a-z]+_get_card)$/;

/**
 * Extract tool call content from ContentPart array
 */
export function extractToolCallContent(content: ContentPart[]): ToolCallContent | null {
  for (const part of content) {
    if (isToolCallPart(part)) {
      return {
        id: part.id,
        name: part.name,
        arguments: part.arguments,
      };
    }
  }
  return null;
}

/**
 * Extract tool result content from ContentPart array
 */
export function extractToolResultContent(content: ContentPart[]): ToolResultContent | null {
  for (const part of content) {
    if (isToolResultPart(part)) {
      return {
        tool_call_id: part.tool_call_id,
        result: part.result,
        error: part.error,
      };
    }
  }
  return null;
}

/**
 * Format arguments as "arg1: value, arg2: value..." with truncation
 */
export function formatArguments(args: Record<string, unknown>): string {
  const entries = Object.entries(args);
  if (entries.length === 0) return "";

  const formatted = entries
    .map(([key, value]) => {
      const strValue = typeof value === "string" ? value : JSON.stringify(value);
      const truncated = strValue.length > 30 ? strValue.substring(0, 30) + "..." : strValue;
      return `${key}: ${truncated}`;
    })
    .join(", ");

  return formatted.length > 80 ? formatted.substring(0, 80) + "..." : formatted;
}

/**
 * Get first N lines of result text, limited to maxChars
 */
export function getResultPreview(
  result: unknown,
  maxLines: number = 2,
  maxChars: number = 360,
): { preview: string; hasMore: boolean } {
  const text = typeof result === "string" ? result : JSON.stringify(result, null, 2);
  const lines = text.split("\n");
  let preview = lines.slice(0, maxLines).join("\n");
  const truncatedByLines = lines.length > maxLines;
  const truncatedByChars = preview.length > maxChars;

  if (truncatedByChars) {
    preview = preview.substring(0, maxChars) + "...";
  }

  return { preview, hasMore: truncatedByLines || truncatedByChars || text.length > preview.length };
}

/**
 * Get full text from ContentPart array (for tool results)
 */
export function getFullText(result: ContentPart[] | undefined): string {
  if (!result || result.length === 0) return "";
  const text = result.flatMap((part) => {
    if (part.type !== "text") return [];
    const embedded = parseMcpContentWrapper(part.text);
    if (!embedded) return [part.text];
    return embedded.flatMap((embeddedPart) => textFromMcpContentPart(embeddedPart) ?? []);
  });
  return text.join("\n");
}

/**
 * Format result content for display
 */
export function formatResult(result: unknown): string {
  return typeof result === "string" ? result : JSON.stringify(result, null, 2);
}

/**
 * toolName must be a first-party `<entity>_get_card` or `mcp_everruns__<entity>_get_card`.
 * Callers that cannot supply a tool name pass undefined, which rejects all cards.
 */
export function extractMcpAppResources(
  result: ContentPart[] | undefined,
  toolName?: string,
): McpAppResource[] {
  if (!result || result.length === 0) return [];
  if (!toolName || !MCP_CARD_TOOL_RE.test(toolName)) return [];

  return result.flatMap((part) => {
    if (part.type === "text") {
      return parseMcpContentWrapper(part.text)?.flatMap(normalizeMcpAppResource) ?? [];
    }
    return normalizeMcpAppResource(part);
  });
}

function parseMcpContentWrapper(text: string): unknown[] | null {
  if (text.length > MCP_CONTENT_WRAPPER_PARSE_LIMIT) return null;
  const trimmed = text.trimStart();
  if (!trimmed.startsWith("{") || !trimmed.includes('"content"')) return null;

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return null;
  }

  if (!isPlainObject(parsed) || !Array.isArray(parsed.content)) return null;
  if (!parsed.content.some(isMcpContentPartLike)) return null;
  return parsed.content;
}

function isMcpContentPartLike(value: unknown): boolean {
  return isPlainObject(value) && (value.type === "text" || value.type === "resource");
}

function textFromMcpContentPart(value: unknown): string | null {
  if (!isPlainObject(value) || value.type !== "text") return null;
  return typeof value.text === "string" ? value.text : null;
}

function normalizeMcpAppResource(value: unknown): McpAppResource[] {
  if (!isPlainObject(value) || value.type !== "resource") return [];

  const nested = isPlainObject(value.resource) ? value.resource : value;
  const uri =
    typeof nested.uri === "string"
      ? nested.uri
      : typeof value.uri === "string"
        ? value.uri
        : undefined;
  const mimeType =
    typeof nested.mimeType === "string"
      ? nested.mimeType
      : typeof nested.mime_type === "string"
        ? nested.mime_type
        : typeof value.mimeType === "string"
          ? value.mimeType
          : typeof value.mime_type === "string"
            ? value.mime_type
            : undefined;
  const html =
    typeof nested.text === "string"
      ? nested.text
      : typeof value.text === "string"
        ? value.text
        : undefined;

  if (!uri || !MCP_CARD_URI_RE.test(uri)) return [];
  if (mimeType?.toLowerCase() !== "text/html") return [];
  if (!html || new TextEncoder().encode(html).length > MCP_CARD_HTML_MAX_BYTES) return [];

  return [{ uri, mimeType, html }];
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
