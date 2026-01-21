"use client";

import { useState } from "react";
import { Check, Loader2, ChevronDown, ChevronRight } from "lucide-react";
import type { ToolCallCompletedData } from "@/lib/api/types";
import { TodoListRenderer, isWriteTodosTool } from "./todo-list-renderer";
import {
  formatArguments,
  getFullText,
  type ToolCallContent,
} from "./tool-call-utils";

interface ToolCallCardFromEventProps {
  toolCall: ToolCallContent;
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

  const statusIcon = isComplete
    ? hasError
      ? <span className="text-red-600 text-xs">✗</span>
      : <Check className="h-3 w-3 text-green-600/80" />
    : <Loader2 className="h-3 w-3 animate-spin text-muted-foreground/60" />;

  const fullText = toolResult?.result ? getFullText(toolResult.result) : "";
  const hasOutput = fullText.length > 0;

  return (
    <div className="text-xs text-muted-foreground/70">
      {/* Tool name with status icon */}
      <div className="flex items-center gap-1">
        {statusIcon}
        <span className="font-mono">{toolCall.name}</span>
        {argsPreview && <span className="opacity-60">{argsPreview}</span>}
      </div>

      {/* Error message */}
      {hasError && (
        <div className="text-red-600 ml-4 mt-0.5">
          Error: {toolResult?.error}
        </div>
      )}

      {/* Expanded output */}
      {isExpanded && hasOutput && (
        <pre className="mt-1 p-1.5 bg-muted/20 rounded text-[10px] leading-tight ml-4 overflow-x-auto max-h-60">
          {fullText}
        </pre>
      )}

      {/* Output toggle */}
      {hasOutput && !hasError && (
        <button
          onClick={() => setIsExpanded(!isExpanded)}
          className="ml-4 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/80 flex items-center gap-0.5"
        >
          {isExpanded ? <ChevronDown className="h-2.5 w-2.5" /> : <ChevronRight className="h-2.5 w-2.5" />}
          {isExpanded ? "hide" : "output"}
        </button>
      )}
    </div>
  );
}
