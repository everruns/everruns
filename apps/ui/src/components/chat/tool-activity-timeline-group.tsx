"use client";

import { useMemo, useState } from "react";
import { AlertCircle, Check, ChevronDown, ChevronRight, Loader2 } from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { getFullText } from "./tool-call-utils";

export interface TimelineToolRow {
  id: string;
  label: string;
  result?: ToolCompletedData;
  state: "running" | "waiting" | "completed" | "error";
}

interface ToolActivityTimelineGroupProps {
  headline: string;
  completedHeadline?: string;
  rows: TimelineToolRow[];
}

function resultPreview(result?: ToolCompletedData): string | null {
  if (!result) return null;
  if (result.error) return result.error;

  const fullText = getFullText(result.result);
  if (!fullText) return null;

  const preview = fullText
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);

  return preview ?? null;
}

function TimelineRow({ row }: { row: TimelineToolRow }) {
  const [expanded, setExpanded] = useState(false);
  const fullText = getFullText(row.result?.result);
  const preview = resultPreview(row.result);
  const hasDetails = fullText.length > 0;

  return (
    <div className="py-1.5">
      <div className="flex items-start gap-2">
        <div className="mt-0.5 flex h-4 w-4 items-center justify-center">
          {row.state === "error" ? (
            <AlertCircle className="h-3.5 w-3.5 text-red-500" />
          ) : row.state === "completed" ? (
            <Check className="h-3.5 w-3.5 text-muted-foreground/75" />
          ) : (
            <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground/80" />
          )}
        </div>

        <div className="min-w-0 flex-1">
          <div className="text-[15px] text-muted-foreground">{row.label}</div>

          {preview && !expanded && (
            <div className="mt-0.5 truncate text-xs text-muted-foreground/75">{preview}</div>
          )}

          {row.result?.error && (
            <div className="mt-0.5 text-xs text-red-600 dark:text-red-400">{row.result.error}</div>
          )}

          {hasDetails && (
            <button
              type="button"
              onClick={() => setExpanded((current) => !current)}
              className="mt-1 inline-flex items-center gap-1 text-[10px] uppercase tracking-[0.18em] text-muted-foreground/70 transition-colors hover:text-foreground"
            >
              {expanded ? (
                <ChevronDown className="h-3 w-3" />
              ) : (
                <ChevronRight className="h-3 w-3" />
              )}
              {expanded ? "hide details" : "details"}
            </button>
          )}

          <div
            className={cn(
              "grid transition-all duration-200 ease-out",
              expanded ? "mt-1 grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
            )}
          >
            <div className="min-h-0 overflow-hidden">
              {hasDetails && (
                <pre className="whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-muted-foreground/90">
                  {fullText}
                </pre>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export function ToolActivityTimelineGroup({
  headline,
  completedHeadline,
  rows,
}: ToolActivityTimelineGroupProps) {
  const completedCount = useMemo(
    () => rows.filter((row) => row.state === "completed" || row.state === "error").length,
    [rows],
  );
  const isActive = completedCount < rows.length;
  const [expanded, setExpanded] = useState(() => isActive);
  const displayHeadline = isActive ? headline : (completedHeadline ?? headline);

  // Single row: render inline with headline as label, no nested wrapper
  if (rows.length === 1) {
    const row = rows[0];
    return (
      <TimelineRow
        row={{ ...row, label: displayHeadline }}
      />
    );
  }

  return (
    <div className="space-y-2">
      <button
        type="button"
        onClick={() => setExpanded((current) => !current)}
        className="inline-flex items-start gap-2 text-left text-muted-foreground transition-colors hover:text-foreground"
        aria-label={expanded ? "Collapse activity group" : "Expand activity group"}
      >
        {expanded ? (
          <ChevronDown className="mt-[2px] h-4 w-4 flex-shrink-0" />
        ) : (
          <ChevronRight className="mt-[2px] h-4 w-4 flex-shrink-0" />
        )}
        <span className="text-[15px] leading-6">{displayHeadline}</span>
      </button>

      <div
        className={cn(
          "grid transition-all duration-200 ease-out",
          expanded ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
        )}
      >
        <div className="min-h-0 overflow-hidden">
          <div className="ml-2 space-y-0.5 border-l border-border/60 pl-3">
            {rows.map((row) => (
              <TimelineRow key={row.id} row={row} />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
