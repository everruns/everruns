"use client";

import { useState } from "react";
import type { Message, ContentPart } from "@/lib/api/types";
import { isToolCallPart, isToolResultPart } from "@/lib/api/types";

interface ToolCallContent {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

interface ToolResultContent {
  tool_call_id: string;
  result?: unknown;
  error?: string;
}

// Extract tool call content from ContentPart array
function extractToolCallContent(content: ContentPart[]): ToolCallContent | null {
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

// Extract tool result content from ContentPart array
function extractToolResultContent(content: ContentPart[]): ToolResultContent | null {
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

// Format arguments as "arg1: value, arg2: value..." with truncation
function formatArguments(args: Record<string, unknown>): string {
  const entries = Object.entries(args);
  if (entries.length === 0) return "";

  const formatted = entries.map(([key, value]) => {
    const strValue = typeof value === "string" ? value : JSON.stringify(value);
    const truncated = strValue.length > 30 ? strValue.substring(0, 30) + "..." : strValue;
    return `${key}: ${truncated}`;
  }).join(", ");

  return formatted.length > 80 ? formatted.substring(0, 80) + "..." : formatted;
}

// Get first N lines of result text, limited to maxChars
function getResultPreview(result: unknown, maxLines: number = 2, maxChars: number = 360): { preview: string; hasMore: boolean } {
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

interface ToolCallCardProps {
  toolCall: Message;
  toolResult?: Message;
}

export function ToolCallCard({ toolCall, toolResult }: ToolCallCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  // Handle new ContentPart[] format
  const content = Array.isArray(toolCall.content)
    ? extractToolCallContent(toolCall.content)
    : (toolCall.content as unknown as ToolCallContent);

  const resultContent = toolResult?.content && Array.isArray(toolResult.content)
    ? extractToolResultContent(toolResult.content)
    : (toolResult?.content as unknown as ToolResultContent | undefined);

  const isComplete = !!toolResult;
  const hasError = resultContent?.error !== undefined && resultContent?.error !== null;

  // Handle missing content gracefully
  if (!content) {
    return null;
  }

  const argsPreview = formatArguments(content.arguments);
  const resultPreview = resultContent?.result !== undefined
    ? getResultPreview(resultContent.result)
    : null;

  return (
    <div className="w-full bg-muted/30 rounded-lg p-3 space-y-1">
      {/* Tool name and arguments */}
      <div className="text-sm">
        <span className="font-semibold">{content.name}:</span>
        {argsPreview && <span className="text-muted-foreground ml-1">{argsPreview}</span>}
      </div>

      {/* Result or executing state */}
      {isComplete ? (
        hasError ? (
          <div className="text-sm text-red-600">
            <span className="text-muted-foreground">&gt;</span> Error: {resultContent?.error}
          </div>
        ) : resultPreview ? (
          <div className="space-y-0.5">
            <pre className="text-sm text-muted-foreground whitespace-pre-wrap">
              <span>&gt; </span>{resultPreview.preview}
            </pre>
            {(resultPreview.hasMore || isExpanded) && (
              <button
                onClick={() => setIsExpanded(!isExpanded)}
                className="text-xs text-blue-600 hover:underline"
              >
                {isExpanded ? "show less" : "> see more..."}
              </button>
            )}
            {isExpanded && resultContent?.result !== undefined && (
              <pre className="text-xs bg-muted/50 p-2 rounded mt-1 overflow-x-auto max-h-60">
                {typeof resultContent.result === "string"
                  ? resultContent.result
                  : JSON.stringify(resultContent.result, null, 2)}
              </pre>
            )}
          </div>
        ) : null
      ) : (
        <div className="text-sm text-muted-foreground">
          <span>&gt;</span> ... executing ...
        </div>
      )}
    </div>
  );
}
