/**
 * Decisions:
 * - Single-row groups collapse into the row component to avoid redundant chrome.
 * - Summary header shows progress counts only; detailed status lives in child rows.
 */
"use client";

import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ToolCompletedData } from "@/lib/api/types";
import type { ToolCallContent } from "./tool-call-utils";
import { summarizeToolCalls } from "./tool-activity-utils";
import { ToolActivityRow } from "./tool-activity-row";

export function GroupedActivityCard({
  toolCalls,
  toolResultsMap,
  mode,
}: {
  toolCalls: ToolCallContent[];
  toolResultsMap: Map<string, ToolCompletedData>;
  mode: "server" | "client";
}) {
  const [isExpanded, setIsExpanded] = useState(true);
  const activityCompletedCount = useMemo(
    () => toolCalls.filter((toolCall) => toolResultsMap.has(toolCall.id)).length,
    [toolCalls, toolResultsMap],
  );
  const isActive = activityCompletedCount < toolCalls.length;

  if (toolCalls.length === 1) {
    const toolCall = toolCalls[0];
    return (
      <ToolActivityRow
        toolCall={toolCall}
        toolResult={toolResultsMap.get(toolCall.id)}
        mode={mode}
      />
    );
  }

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-3 py-1">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <div className="text-[11px] uppercase tracking-[0.22em] text-muted-foreground/70">
              {summarizeToolCalls(toolCalls)}
            </div>
            <div className="text-[10px] text-muted-foreground/50">
              {activityCompletedCount}/{toolCalls.length}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {isActive ? (
            <div className="flex items-center gap-2 text-[10px] uppercase tracking-[0.22em] text-accent-foreground/70">
              <span className="animate-tool-pulse inline-flex h-1.5 w-1.5 bg-accent" />
            </div>
          ) : null}
          <button
            type="button"
            onClick={() => setIsExpanded((current) => !current)}
            className="rounded p-1 text-muted-foreground/60 transition-colors hover:bg-muted hover:text-foreground"
            aria-label={isExpanded ? "Collapse tool activity" : "Expand tool activity"}
          >
            {isExpanded ? (
              <ChevronDown className="h-4 w-4" />
            ) : (
              <ChevronRight className="h-4 w-4" />
            )}
          </button>
        </div>
      </div>

      <div
        className={cn(
          "grid transition-all duration-300 ease-out",
          isExpanded ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
        )}
      >
        <div className="min-h-0 overflow-hidden">
          <div className="space-y-1 border-l border-border/60 pl-3">
            {toolCalls.map((toolCall) => (
              <ToolActivityRow
                key={toolCall.id}
                toolCall={toolCall}
                toolResult={toolResultsMap.get(toolCall.id)}
                mode={mode}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
