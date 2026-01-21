"use client";

import { useState } from "react";
import type { Message } from "@/lib/api/types";
import { TodoListRenderer, isWriteTodosTool } from "./todo-list-renderer";
import {
  extractToolCallContent,
  extractToolResultContent,
  formatArguments,
  getResultPreview,
  formatResult,
  type ToolCallContent,
  type ToolResultContent,
} from "./tool-call-utils";

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

  // Special rendering for write_todos tool
  if (isWriteTodosTool(content.name)) {
    return (
      <div className="w-full">
        <TodoListRenderer
          arguments={content.arguments}
          result={resultContent?.result}
          isExecuting={!isComplete}
          error={resultContent?.error}
        />
      </div>
    );
  }

  return (
    <div className="w-full space-y-0.5 text-sm text-muted-foreground">
      {/* Tool name and arguments */}
      <div>
        <span className="font-medium">{content.name}:</span>
        {argsPreview && <span className="ml-1">{argsPreview}</span>}
      </div>

      {/* Result or executing state */}
      {isComplete ? (
        hasError ? (
          <div className="text-red-600">
            &gt; Error: {resultContent?.error}
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
            {isExpanded && resultContent?.result !== undefined && (
              <pre className="text-xs bg-muted/50 p-2 rounded mt-1 overflow-x-auto max-h-60">
                {formatResult(resultContent.result)}
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
