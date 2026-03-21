/**
 * Decisions:
 * - Keep compaction metadata inline in the transcript so users can inspect context loss without leaving chat.
 * - Default collapsed; expanded details are diagnostic, not primary reading flow.
 */
"use client";

import { useState } from "react";
import type { ContextCompactedData } from "@/lib/api/types";

export function CompactionDivider({ data }: { data: ContextCompactedData }) {
  const [expanded, setExpanded] = useState(false);
  const saved = data.messages_before - data.messages_after;

  return (
    <div className="space-y-1">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="flex w-full cursor-pointer items-center gap-4 py-2 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground sm:text-sm"
      >
        <div className="h-px flex-1 bg-border" />
        <span className="whitespace-nowrap">
          Context compacted · {data.messages_before} → {data.messages_after} messages
          {data.strategy_used !== "none" && ` · ${data.strategy_used}`}
        </span>
        <span className="text-[10px]">{expanded ? "▲" : "▼"}</span>
        <div className="h-px flex-1 bg-border" />
      </button>
      {expanded && (
        <div className="mx-auto max-w-md rounded-md border bg-muted/50 px-4 py-2 text-xs text-muted-foreground">
          <div className="space-y-1">
            <div>
              <span className="font-medium">Saved:</span> {saved} messages in {data.duration_ms}ms
            </div>
            {data.steps.length > 0 && (
              <div>
                <span className="font-medium">Cascade steps:</span>
                <ol className="mt-1 list-inside list-decimal space-y-0.5">
                  {data.steps.map((step, index) => (
                    <li key={index}>
                      {step.strategy} → {step.messages_after} messages ({step.duration_ms}ms)
                    </li>
                  ))}
                </ol>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
