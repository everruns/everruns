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
  return result
    .filter((part): part is { type: "text"; text: string } => part.type === "text")
    .map((part) => part.text)
    .join("\n");
}

/**
 * Format result content for display
 */
export function formatResult(result: unknown): string {
  return typeof result === "string" ? result : JSON.stringify(result, null, 2);
}
