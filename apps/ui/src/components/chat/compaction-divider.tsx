/**
 * Decisions:
 * - Keep compaction metadata inline in the transcript so users can inspect context loss without leaving chat.
 * - Default collapsed; expanded details are diagnostic, not primary reading flow.
 * - Zero messages on both sides means the counts are unknown (a provider compacted context it
 *   holds, EVE-1125), so no "0 → 0" claim is shown.
 * - Native compaction replaces the transcript with the provider's output items, and
 *   `messages_after` counts those items, not messages, so it can exceed `messages_before`.
 *   Show its token counts instead, and no message counts when tokens are missing.
 */
"use client";

import { useState } from "react";
import type { ContextCompactedData } from "@/lib/api/types";
import { formatCompactNumber } from "@/lib/formatting";

/** The event as the wire sends it: token counts ride along when measured. */
export type CompactionMarkerData = ContextCompactedData & {
  tokens_before?: number;
  tokens_after?: number;
};

export function CompactionDivider({ data }: { data: CompactionMarkerData }) {
  const [expanded, setExpanded] = useState(false);
  const native = data.strategy_used.split("+").includes("native");
  const tokensKnown = data.tokens_before != null && data.tokens_after != null;
  const saved = data.messages_before - data.messages_after;
  const countsKnown = !native && (data.messages_before > 0 || data.messages_after > 0);
  // `steps` is omitted on the wire when empty (provider-managed compaction has none).
  const steps = data.steps ?? [];

  return (
    <div className="space-y-1">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        className="flex w-full cursor-pointer items-center gap-4 py-2 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground sm:text-sm"
      >
        <div className="h-px flex-1 bg-border" />
        <span className="whitespace-nowrap">
          Context compacted
          {countsKnown && ` · ${data.messages_before} → ${data.messages_after} messages`}
          {native &&
            tokensKnown &&
            ` · ${formatCompactNumber(data.tokens_before!)} → ${formatCompactNumber(data.tokens_after!)} tokens`}
          {data.strategy_used !== "none" && ` · ${data.strategy_used}`}
        </span>
        <span className="text-[10px]">{expanded ? "▲" : "▼"}</span>
        <div className="h-px flex-1 bg-border" />
      </button>
      {expanded && (
        <div className="mx-auto max-w-md border bg-muted/50 px-4 py-2 text-xs text-muted-foreground">
          <div className="space-y-1">
            {countsKnown ? (
              <div>
                <span className="font-medium">Saved:</span> {saved} messages in {data.duration_ms}ms
              </div>
            ) : (
              <div>
                <span className="font-medium">Strategy:</span> {data.strategy_used}
              </div>
            )}
            {steps.length > 0 && (
              <div>
                <span className="font-medium">Cascade steps:</span>
                <ol className="mt-1 list-inside list-decimal space-y-0.5">
                  {steps.map((step, index) => (
                    <li key={index}>
                      {step.strategy === "native"
                        ? `native (${step.duration_ms}ms)`
                        : `${step.strategy} → ${step.messages_after} messages (${step.duration_ms}ms)`}
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
