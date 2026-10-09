/**
 * Decisions:
 * - Row state derives from tool result presence; no duplicate status enum needed.
 * - Expanded output reuses the same formatter as previews to keep masking rules aligned.
 * - Result images (computer-use screenshots) render as thumbnails outside the
 *   collapsible details, so the frame an action produced is visible at a glance.
 */
"use client";

import { useState } from "react";
import { CalendarClock, Check, Loader2, MonitorSmartphone } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ToolCompletedData } from "@/lib/api/types";
import { ExecutedArgumentsNotice } from "./executed-arguments";
import { FailureIcon, failureTextClass, toolFailureSeverity } from "./failure-severity";
import { McpAppResourceList } from "./mcp-app-resource-list";
import { ToolResultThumbnails } from "./tool-result-thumbnails";
import type { ToolCallContent } from "./tool-call-utils";
import {
  formatResultDetails,
  getToolActivitySummaryChip,
  getToolLabel,
} from "./tool-activity-utils";
import { extractMcpAppResources, extractResultImages, getFullText } from "./tool-call-utils";
import { useLocale } from "@/providers/locale-provider";
import { ApprovalToolActivity, type ApprovalToolContext } from "./approval-tool-activity";

export function ToolActivityRow({
  toolCall,
  toolResult,
  progressMessage,
  mode,
  locale,
  approvalContext,
}: {
  toolCall: ToolCallContent;
  toolResult?: ToolCompletedData;
  /** Latest tool.progress message for this tool call */
  progressMessage?: string;
  mode: "server" | "client";
  locale: string;
  approvalContext?: ApprovalToolContext;
}) {
  const { t } = useLocale();
  const [isExpanded, setIsExpanded] = useState(false);
  const fullText = getFullText(toolResult?.result);
  const mcpAppResources = extractMcpAppResources(toolResult?.result, toolCall.name);
  const resultImages = extractResultImages(toolResult?.result);
  const detailsId = `tool-activity-details-${toolCall.id}`;
  const hasOutput = fullText.length > 0;
  const hasToolError = Boolean(toolResult?.error);
  const failureSeverity = hasToolError ? (toolFailureSeverity(toolResult) ?? "issue") : null;
  const isComplete = Boolean(toolResult);
  const isRunning = !isComplete && !hasToolError;
  const summaryChip = isComplete ? getToolActivitySummaryChip(toolCall, toolResult) : null;

  if (toolCall.name === "request_approval" || toolCall.name === "record_approval") {
    return (
      <ApprovalToolActivity toolCall={toolCall} toolResult={toolResult} context={approvalContext} />
    );
  }

  return (
    <div
      className={cn(
        "animate-tool-row-in py-2 transition-all duration-300",
        isRunning && "border-l border-l-accent/70 bg-[hsl(var(--accent)/0.05)] pl-2.5",
      )}
    >
      <div className="flex items-start gap-2">
        <div className="mt-0.5 flex h-4 w-4 items-center justify-center">
          {failureSeverity ? (
            <FailureIcon severity={failureSeverity} />
          ) : isComplete ? (
            <Check className="h-3.5 w-3.5 text-accent" />
          ) : (
            <Loader2 className="h-3.5 w-3.5 animate-spin text-accent" />
          )}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            {mode === "client" && (
              <MonitorSmartphone className="mt-0.5 h-3.5 w-3.5 flex-shrink-0 text-primary/75" />
            )}
            {summaryChip ? (
              <div className="inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-[8px] border border-border/70 bg-background px-2.5 py-1 text-[13px] shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
                <CalendarClock className="h-3.5 w-3.5 flex-shrink-0 text-muted-foreground" />
                <span className="shrink-0 text-muted-foreground">{summaryChip.status}</span>
                <span className="shrink-0 text-muted-foreground/60">·</span>
                <span className="truncate text-primary">{summaryChip.title}</span>
                {summaryChip.schedule && (
                  <>
                    <span className="shrink-0 text-muted-foreground/60">·</span>
                    <span className="shrink-0 text-muted-foreground">{summaryChip.schedule}</span>
                  </>
                )}
              </div>
            ) : (
              <span className="truncate text-sm text-foreground">
                {getToolLabel(toolCall, locale)}
              </span>
            )}
            {isRunning && (
              <span className="animate-tool-pulse text-[10px] uppercase tracking-[0.18em] text-accent-foreground/70">
                {progressMessage ?? (mode === "client" ? t("waiting") : t("running"))}
              </span>
            )}
            {!isRunning && hasOutput && !summaryChip && (
              <button
                type="button"
                onClick={() => setIsExpanded((current) => !current)}
                aria-expanded={isExpanded}
                aria-controls={detailsId}
                className="inline-flex items-center gap-1 text-[10px] uppercase tracking-[0.18em] text-muted-foreground/50 transition-colors hover:text-foreground"
              >
                {isExpanded ? t("hide_details") : t("details")}
              </button>
            )}
          </div>

          {failureSeverity && (
            <div className={cn("mt-1 text-xs", failureTextClass(failureSeverity))}>
              {toolResult?.error}
            </div>
          )}

          <ExecutedArgumentsNotice toolResult={toolResult} originalArguments={toolCall.arguments} />

          <McpAppResourceList resources={mcpAppResources} />

          <ToolResultThumbnails images={resultImages} />

          <div
            id={detailsId}
            className={cn(
              "grid transition-all duration-300 ease-out",
              isExpanded ? "mt-1.5 grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
            )}
          >
            <div className="min-h-0 overflow-hidden">
              {hasOutput && (
                <pre className="max-h-80 overflow-x-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-muted-foreground/85">
                  {formatResultDetails(toolCall, fullText)}
                </pre>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
