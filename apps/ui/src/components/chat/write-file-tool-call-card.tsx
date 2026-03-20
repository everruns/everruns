"use client";

/**
 * Decisions:
 * - Write-like file mutations should stay inline, matching bash/read_file instead of reopening the grouped activity chrome.
 * - Structured JSON tool output is reduced to a compact status line so file writes read like transcript events, not raw payload dumps.
 * - When a write tool returns freeform text instead of metadata, keep an expandable fallback instead of forcing JSON-only handling.
 */

import { useMemo, useState } from "react";
import { Check, ChevronDown, ChevronRight, FilePenLine, Loader2 } from "lucide-react";
import type { ContentPart, ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { basename } from "@/lib/path-utils";
import { formatFileSize } from "@/lib/formatting";
import { getFullText, type ToolCallContent } from "./tool-call-utils";

// Re-export from centralized registry for backward-compatible imports.
export { isWriteLikeTool } from "@/lib/tool-registry";

interface WriteFileToolCallCardProps {
  toolCall: ToolCallContent;
  toolResult?: ToolCompletedData;
}

interface WriteFilePayload {
  path?: string;
  size_bytes?: number;
  created?: boolean;
}

interface ParsedWriteFileResult {
  path?: string;
  sizeBytes?: number;
  created?: boolean;
  textContent: string | null;
}

function getPathFromArguments(toolCall: ToolCallContent): string | undefined {
  const path = toolCall.arguments.path;
  return typeof path === "string" && path.trim().length > 0 ? path : undefined;
}

function parseWriteFilePayload(
  toolCall: ToolCallContent,
  result: ContentPart[] | undefined,
): ParsedWriteFileResult {
  const rawText = getFullText(result);
  try {
    const parsed = JSON.parse(rawText) as WriteFilePayload;
    if (typeof parsed === "object" && parsed !== null) {
      return {
        path:
          (typeof parsed.path === "string" && parsed.path.length > 0 ? parsed.path : undefined) ??
          getPathFromArguments(toolCall),
        sizeBytes: typeof parsed.size_bytes === "number" ? parsed.size_bytes : undefined,
        created: typeof parsed.created === "boolean" ? parsed.created : undefined,
        textContent: null,
      };
    }
  } catch {
    // Fall back to raw text below.
  }

  return {
    path: getPathFromArguments(toolCall),
    sizeBytes: undefined,
    created: undefined,
    textContent: rawText || null,
  };
}

function getToolVerb(toolName: string): string {
  switch (toolName) {
    case "edit_file":
      return "Edit";
    case "replace_in_file":
      return "Replace in";
    case "append_file":
      return "Append to";
    default:
      return "Write";
  }
}

function getTextPreview(text: string | null): string | null {
  if (!text) return null;
  const firstLine = text
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  if (!firstLine) return null;
  return firstLine.length > 120 ? `${firstLine.slice(0, 120)}...` : firstLine;
}

// isWriteLikeTool is re-exported from @/lib/tool-registry at the top of this file.

export function WriteFileToolCallCard({ toolCall, toolResult }: WriteFileToolCallCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const parsed = useMemo(
    () => parseWriteFilePayload(toolCall, toolResult?.result),
    [toolCall, toolResult?.result],
  );

  const isComplete = !!toolResult;
  const hasError = toolResult?.error !== undefined && toolResult?.error !== null;
  const resolvedPath = parsed.path ?? getPathFromArguments(toolCall);
  const title = resolvedPath
    ? `${getToolVerb(toolCall.name)} ${basename(resolvedPath)}`
    : `${getToolVerb(toolCall.name)} file`;
  const durationLabel = toolResult?.duration_ms
    ? toolResult.duration_ms < 1000
      ? `${toolResult.duration_ms}ms`
      : `${(toolResult.duration_ms / 1000).toFixed(1)}s`
    : null;
  const metadataPreview = [parsed.created ? "created" : "updated", formatFileSize(parsed.sizeBytes)]
    .filter(Boolean)
    .join(" · ");
  const preview = metadataPreview || getTextPreview(parsed.textContent);
  const hasTextDetails = !!parsed.textContent;

  const statusIcon = isComplete ? (
    hasError ? (
      <span className="text-red-500 text-xs font-bold">!</span>
    ) : (
      <Check className="h-3 w-3 text-green-600/80" />
    )
  ) : (
    <Loader2 className="h-3 w-3 animate-spin text-muted-foreground/60" />
  );

  return (
    <div className="text-xs text-muted-foreground/70">
      <div className="flex items-center gap-1.5">
        {statusIcon}
        <FilePenLine className="h-3 w-3 opacity-50" />
        <button
          type="button"
          onClick={() => hasTextDetails && setIsExpanded((current) => !current)}
          className={cn(
            "flex min-w-0 items-center gap-1",
            hasTextDetails ? "cursor-pointer hover:text-muted-foreground" : "cursor-default",
          )}
        >
          <span className="truncate text-sm text-foreground">{title}</span>
        </button>
        {durationLabel && (
          <span className="flex-shrink-0 text-[10px] opacity-40">{durationLabel}</span>
        )}
        {hasTextDetails && (
          <button
            type="button"
            onClick={() => setIsExpanded((current) => !current)}
            className="flex-shrink-0 opacity-40 transition-opacity hover:opacity-70"
            aria-label={isExpanded ? "Collapse write output" : "Expand write output"}
          >
            {isExpanded ? (
              <ChevronDown className="h-3 w-3" />
            ) : (
              <ChevronRight className="h-3 w-3" />
            )}
          </button>
        )}
      </div>

      {hasError && (
        <div className="ml-[22px] mt-0.5 text-[10px] text-red-600">Error: {toolResult?.error}</div>
      )}

      {!hasError && preview && !isExpanded && (
        <div className="ml-[22px] mt-0.5 truncate text-[10px] opacity-70">{preview}</div>
      )}

      {!hasError && isExpanded && parsed.textContent && (
        <div className="ml-[22px] mt-1.5">
          <pre className="overflow-x-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-muted-foreground/85">
            {parsed.textContent}
          </pre>
        </div>
      )}
    </div>
  );
}
