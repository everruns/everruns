"use client";

import { useState } from "react";
import { Check, Loader2, ChevronDown, ChevronRight } from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import { useLocale } from "@/providers/locale-provider";
import { TodoListRenderer, isWriteTodosTool } from "./todo-list-renderer";
import { BashToolCallCard, isBashTool } from "./bash-tool-call-card";
import { ReadFileToolCallCard, isReadFileTool } from "./read-file-tool-call-card";
import { formatArguments, getFullText, type ToolCallContent } from "./tool-call-utils";
import { ClientToolCallCard } from "./client-tool-call-card";
import { WriteFileToolCallCard, isWriteLikeTool } from "./write-file-tool-call-card";

interface ToolCallCardFromEventProps {
  toolCall: ToolCallContent;
  toolResult?: ToolCompletedData;
  /** When true, renders as a client-side tool call with distinct styling */
  isClientSide?: boolean;
}

/**
 * Render a tool call card from event data
 * Uses event-based data format (ToolCompletedData) instead of Message format.
 * Supports both server-side and client-side tool calls.
 */
export function ToolCallCardFromEvent({
  toolCall,
  toolResult,
  isClientSide,
}: ToolCallCardFromEventProps) {
  const { t } = useLocale();
  const [isExpanded, setIsExpanded] = useState(false);

  // Delegate to ClientToolCallCard for client-side tool calls
  if (isClientSide) {
    return <ClientToolCallCard toolCall={toolCall} toolResult={toolResult} />;
  }

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

  // Special rendering for bash/shell tool
  if (isBashTool(toolCall.name)) {
    return <BashToolCallCard toolCall={toolCall} toolResult={toolResult} />;
  }

  if (isReadFileTool(toolCall.name)) {
    return <ReadFileToolCallCard toolCall={toolCall} toolResult={toolResult} />;
  }

  if (isWriteLikeTool(toolCall.name)) {
    return <WriteFileToolCallCard toolCall={toolCall} toolResult={toolResult} />;
  }

  const statusIcon = isComplete ? (
    hasError ? (
      <span className="text-red-600 text-xs">✗</span>
    ) : (
      <Check className="h-3 w-3 text-green-600/80" />
    )
  ) : (
    <Loader2 className="h-3 w-3 animate-spin text-muted-foreground/60" />
  );

  const fullText = toolResult?.result ? getFullText(toolResult.result) : "";
  const hasOutput = fullText.length > 0;

  return (
    <div className="text-xs text-muted-foreground/70">
      {/* Tool name with status icon */}
      <div className="flex items-center gap-1">
        {statusIcon}
        <span className="font-mono">
          {toolResult?.display_name ?? toolCall.display_name ?? toolCall.name}
        </span>
        {argsPreview && <span className="opacity-60">{argsPreview}</span>}
      </div>

      {/* Error message */}
      {hasError && (
        <div className="text-red-600 ml-4 mt-0.5">
          {t("error_prefix", { value: toolResult?.error ?? "" })}
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
          {isExpanded ? (
            <ChevronDown className="h-2.5 w-2.5" />
          ) : (
            <ChevronRight className="h-2.5 w-2.5" />
          )}
          {isExpanded ? t("hide_details") : t("output")}
        </button>
      )}
    </div>
  );
}
