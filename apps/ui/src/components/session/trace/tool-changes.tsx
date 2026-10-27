"use client";

// Tool change recorded for one model call. Models that take changes inline
// keep the request's tool list fixed and get the change as a message, so the
// prompt cache survives it; other models get the new tool list.

import { useUiTranslations } from "@/providers/locale-provider";
import type { TraceToolChanges } from "@/lib/api/types";
import { formatCount } from "./trace-model";

export function ToolChanges({ changes }: { changes: TraceToolChanges }) {
  const ui = useUiTranslations();
  const parts = [
    ["Added", changes.added],
    ["Updated", changes.redefined],
    ["Removed", changes.removed],
  ] as const;
  return (
    <div className="mb-3 space-y-0.5 text-[11px]" role="note">
      {changes.baseline_tools != null && (
        <p className="text-muted-foreground">
          {ui("Tool list fixed for this session:")} {formatCount(changes.baseline_tools)}{" "}
          {ui("tools. Later changes are recorded instead of rewriting it.")}
        </p>
      )}
      {parts.map(([label, names]) =>
        names.length > 0 ? (
          <p key={label}>
            <span className="text-muted-foreground">
              {ui("Tools")} {label.toLowerCase()}:{" "}
            </span>
            <span className="font-mono">{names.join(", ")}</span>
          </p>
        ) : null,
      )}
    </div>
  );
}
