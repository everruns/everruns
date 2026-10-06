"use client";

/**
 * Decisions:
 * - `tool.completed.executed_arguments` exists only when a `pre_tool_use` hook rewrote the
 *   model-authored arguments, so its presence alone drives the indicator; no client-side diffing.
 * - The value is untyped: a JSON value, or a truncated JSON string past the server budget
 *   (`executed_arguments_truncated`). Strings render verbatim; everything else pretty-prints.
 * - Values arrive already redacted server-side (`[REDACTED]`); render as-is.
 * - The indicator owns its own toggle so every tool card can drop it in without threading
 *   state through each card's details panel.
 */

import { useId, useState } from "react";
import { ChevronDown, ChevronRight, Webhook } from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { useLocale } from "@/providers/locale-provider";

export interface ExecutedArguments {
  text: string;
  truncated: boolean;
}

function formatArgumentsValue(value: unknown): string {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value, null, 2) ?? String(value);
  } catch {
    return String(value);
  }
}

/** Hook-rewritten arguments from a `tool.completed` result, or null when the call ran as authored. */
export function getExecutedArguments(toolResult?: ToolCompletedData): ExecutedArguments | null {
  if (toolResult?.executed_arguments == null) return null;
  return {
    text: formatArgumentsValue(toolResult.executed_arguments),
    truncated: toolResult.executed_arguments_truncated === true,
  };
}

interface ExecutedArgumentsNoticeProps {
  toolResult?: ToolCompletedData;
  /** Model-authored arguments (from the tool call / `tool.started`), when known. */
  originalArguments?: Record<string, unknown>;
  className?: string;
}

export function ExecutedArgumentsNotice({
  toolResult,
  originalArguments,
  className,
}: ExecutedArgumentsNoticeProps) {
  const { t } = useLocale();
  const [expanded, setExpanded] = useState(false);
  const panelId = useId();
  const executed = getExecutedArguments(toolResult);
  if (!executed) return null;

  const hasOriginal = originalArguments !== undefined && Object.keys(originalArguments).length > 0;

  return (
    <div className={cn("mt-1 text-xs", className)} data-testid="executed-arguments">
      <button
        type="button"
        onClick={() => setExpanded((current) => !current)}
        aria-expanded={expanded}
        aria-controls={panelId}
        className="inline-flex items-center gap-1 rounded-[6px] border border-border/70 px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:text-foreground"
      >
        <Webhook className="h-3 w-3 flex-shrink-0" aria-hidden="true" />
        {t("arguments_rewritten_by_hook")}
        {expanded ? (
          <ChevronDown className="h-3 w-3 flex-shrink-0" aria-hidden="true" />
        ) : (
          <ChevronRight className="h-3 w-3 flex-shrink-0" aria-hidden="true" />
        )}
      </button>

      {expanded && (
        <div id={panelId} className="mt-1.5 space-y-2">
          {hasOriginal && (
            <div>
              <div className="text-[10px] uppercase tracking-[0.18em] text-muted-foreground/70">
                {t("original_arguments")}
              </div>
              <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-muted-foreground/85">
                {formatArgumentsValue(originalArguments)}
              </pre>
            </div>
          )}
          <div>
            <div className="text-[10px] uppercase tracking-[0.18em] text-muted-foreground/70">
              {t("executed_arguments")}
            </div>
            <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-foreground/90">
              {executed.text}
            </pre>
            {executed.truncated && (
              <div className="mt-0.5 text-[10px] text-muted-foreground/70">
                {t("executed_arguments_truncated")}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
