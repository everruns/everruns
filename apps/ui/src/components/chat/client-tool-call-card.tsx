"use client";

import { useState } from "react";
import { Check, Loader2, ChevronDown, ChevronRight, MonitorSmartphone } from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import { formatArguments, getFullText, type ToolCallContent } from "./tool-call-utils";

interface ClientToolCallCardProps {
  toolCall: ToolCallContent;
  toolResult?: ToolCompletedData;
}

/**
 * Render a client-side tool call card.
 * Distinct from server-side tool calls: uses a device icon and amber color
 * to indicate the tool runs on the client, not the server.
 */
export function ClientToolCallCard({ toolCall, toolResult }: ClientToolCallCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  const isComplete = !!toolResult;
  const hasError = toolResult?.error !== undefined && toolResult?.error !== null;

  const argsPreview = formatArguments(toolCall.arguments);

  const statusIcon = isComplete ? (
    hasError ? (
      <span className="text-red-600 text-xs">&#x2717;</span>
    ) : (
      <Check className="h-3 w-3 text-green-600/80" />
    )
  ) : (
    <Loader2 className="h-3 w-3 animate-spin text-amber-500/70" />
  );

  const fullText = toolResult?.result ? getFullText(toolResult.result) : "";
  const hasOutput = fullText.length > 0;

  return (
    <div className="text-xs text-muted-foreground/70">
      {/* Tool name with client-side indicator */}
      <div className="flex items-center gap-1">
        {statusIcon}
        <MonitorSmartphone className="h-3 w-3 text-amber-500/70" />
        <span className="font-mono text-amber-700 dark:text-amber-400">{toolCall.name}</span>
        {!isComplete && (
          <span className="text-amber-500/70 italic text-[10px] ml-1">Waiting for client...</span>
        )}
        {argsPreview && <span className="opacity-60">{argsPreview}</span>}
      </div>

      {/* Error message */}
      {hasError && <div className="text-red-600 ml-4 mt-0.5">Error: {toolResult?.error}</div>}

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
          {isExpanded ? "hide" : "output"}
        </button>
      )}
    </div>
  );
}
