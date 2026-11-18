/**
 * Helpers for the unified diff an edit_file call returns.
 *
 * Decisions:
 * - Kept apart from EditDiffView so the collapsed card can show +/− counts without loading
 *   the diff renderer (@pierre/diffs), which only loads when a card is expanded.
 * - The server labels both sides "<path> (before)" / "<path> (after)"
 *   (`render_unified_diff` in crates/integrations/src/filesystem/mod.rs). The labels are
 *   stripped so the renderer picks the language from the real file extension, and the
 *   truncation marker `truncate_diff` appends is dropped because it is not diff syntax.
 */

import { useMemo } from "react";
import { cn } from "@/lib/utils";

const TRUNCATION_MARKER = /\n\.\.\. diff truncated after \d+ characters \.\.\.\s*$/;

export function normalizeEditPatch(diff: string): string {
  return diff
    .replace(TRUNCATION_MARKER, "\n")
    .replace(/^(---|\+\+\+) (.*) \((before|after)\)$/gm, "$1 $2");
}

export function countDiffLines(diff: string): {
  added: number;
  removed: number;
} {
  let added = 0;
  let removed = 0;
  for (const line of normalizeEditPatch(diff).split("\n")) {
    if (line.startsWith("+++ ") || line.startsWith("--- ")) continue;
    if (line.startsWith("+")) added += 1;
    else if (line.startsWith("-")) removed += 1;
  }
  return { added, removed };
}

export function DiffCounts({ diff, className }: { diff: string; className?: string }) {
  const { added, removed } = useMemo(() => countDiffLines(diff), [diff]);
  return (
    <span className={cn("font-mono tabular-nums", className)}>
      <span className="text-success">+{added}</span>{" "}
      <span className="text-destructive">−{removed}</span>
    </span>
  );
}
