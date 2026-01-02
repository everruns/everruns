"use client";

import { useState } from "react";
import type { ToolCallCompletedData, ContentPart } from "@/lib/api/types";
import { TodoListRenderer, isWriteTodosTool } from "./todo-list-renderer";

interface ToolCallInfo {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
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
function getResultPreview(result: ContentPart[] | undefined, maxLines: number = 2, maxChars: number = 360): { preview: string; hasMore: boolean } | null {
  if (!result || result.length === 0) return null;

  // Extract text from ContentPart array
  const text = result
    .filter((part): part is { type: "text"; text: string } => part.type === "text")
    .map(part => part.text)
    .join("\n");

  if (!text) return null;

  const lines = text.split("\n");
  let preview = lines.slice(0, maxLines).join("\n");
  const truncatedByLines = lines.length > maxLines;
  const truncatedByChars = preview.length > maxChars;

  if (truncatedByChars) {
    preview = preview.substring(0, maxChars) + "...";
  }

  return { preview, hasMore: truncatedByLines || truncatedByChars || text.length > preview.length };
}

// Get full text from ContentPart array
function getFullText(result: ContentPart[] | undefined): string {
  if (!result || result.length === 0) return "";
  return result
    .filter((part): part is { type: "text"; text: string } => part.type === "text")
    .map(part => part.text)
    .join("\n");
}

interface ToolCallCardFromEventProps {
  toolCall: ToolCallInfo;
  toolResult?: ToolCallCompletedData;
}

/**
 * Render a tool call card from event data
 * Uses event-based data format (ToolCallCompletedData) instead of Message format
 */
export function ToolCallCardFromEvent({ toolCall, toolResult }: ToolCallCardFromEventProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  const isComplete = !!toolResult;
  const hasError = toolResult?.error !== undefined && toolResult?.error !== null;

  const argsPreview = formatArguments(toolCall.arguments);
  const resultPreview = toolResult?.result
    ? getResultPreview(toolResult.result)
    : null;

  // Special rendering for write_todos tool
  if (isWriteTodosTool(toolCall.name)) {
    return (
      <div className="w-full">
        <TodoListRenderer
          arguments={toolCall.arguments}
          result={toolResult?.result}
          isExecuting={!isComplete}
          error={toolResult?.error}
        />
      </div>
    );
  }

  return (
    <div className="w-full space-y-0.5 text-sm text-muted-foreground">
      {/* Tool name and arguments */}
      <div>
        <span className="font-medium">{toolCall.name}:</span>
        {argsPreview && <span className="ml-1">{argsPreview}</span>}
      </div>

      {/* Result or executing state */}
      {isComplete ? (
        hasError ? (
          <div className="text-red-600">
            &gt; Error: {toolResult?.error}
          </div>
        ) : resultPreview ? (
          <div className="space-y-0.5">
            <div className="whitespace-pre-wrap">
              &gt; {resultPreview.preview}
            </div>
            {(resultPreview.hasMore || isExpanded) && (
              <button
                onClick={() => setIsExpanded(!isExpanded)}
                className="text-xs text-blue-600 hover:underline"
              >
                {isExpanded ? "show less" : "> see more..."}
              </button>
            )}
            {isExpanded && toolResult?.result && (
              <pre className="text-xs bg-muted/50 p-2 rounded mt-1 overflow-x-auto max-h-60">
                {getFullText(toolResult.result)}
              </pre>
            )}
          </div>
        ) : null
      ) : (
        <div>
          &gt; ... executing ...
        </div>
      )}
    </div>
  );
}
