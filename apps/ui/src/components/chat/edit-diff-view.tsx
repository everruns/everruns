"use client";

/**
 * EditDiffView - syntax-highlighted unified diff for an edit_file result.
 *
 * Decisions:
 * - Rendered with @pierre/diffs (Apache-2.0). It shares the Shiki version Streamdown already
 *   ships, and the card loads this module only when the user expands an edit.
 * - The tool card already names the file, so the library's file header is off.
 * - Same Shiki themes as Streamdown code blocks (github-light / github-dark), following the
 *   console theme rather than the OS setting.
 * - Rendered in place, without the library's worker pool: tool diffs are capped server-side
 *   (MAX_EDIT_DIFF_CHARS), so a worker would cost more than it saves.
 */

import { PatchDiff } from "@pierre/diffs/react";
import { useMemo, type CSSProperties } from "react";
import { useTheme } from "@/providers/theme-provider";
import { cn } from "@/lib/utils";
import { normalizeEditPatch } from "./edit-diff";

const DIFF_STYLE_VARS = {
  "--diffs-font-size": "12px",
  "--diffs-line-height": "18px",
  "--diffs-font-family": "var(--font-mono)",
  "--diffs-header-font-family": "var(--font-sans)",
} as CSSProperties;

export default function EditDiffView({ diff, className }: { diff: string; className?: string }) {
  const { theme } = useTheme();
  const patch = useMemo(() => normalizeEditPatch(diff), [diff]);
  return (
    <div
      className={cn("max-h-[480px] overflow-auto rounded-md border border-border/60", className)}
      style={DIFF_STYLE_VARS}
      data-testid="edit-diff-view"
    >
      <PatchDiff
        patch={patch}
        disableWorkerPool
        options={{
          theme: { light: "github-light", dark: "github-dark" },
          themeType: theme,
          diffStyle: "unified",
          disableFileHeader: true,
          diffIndicators: "bars",
          lineDiffType: "word",
          hunkSeparators: "line-info-basic",
          overflow: "scroll",
        }}
      />
    </div>
  );
}
