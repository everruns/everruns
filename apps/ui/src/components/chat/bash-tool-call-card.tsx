"use client";

import { useState } from "react";
import { Check, Loader2, ChevronDown, ChevronRight, Terminal } from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { getFullText, type ToolCallContent } from "./tool-call-utils";

interface BashToolCallCardProps {
  toolCall: ToolCallContent;
  toolResult?: ToolCompletedData;
}

export interface BashOutput {
  stdout: string;
  stderr: string;
  exit_code: number;
  success: boolean;
}

/**
 * Try to parse bash JSON output: {"stdout":"...","stderr":"...","exit_code":0,"success":true}
 */
export function parseBashOutput(text: string): BashOutput | null {
  try {
    const parsed = JSON.parse(text);
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      "stdout" in parsed &&
      "exit_code" in parsed
    ) {
      return {
        stdout: String(parsed.stdout ?? ""),
        stderr: String(parsed.stderr ?? ""),
        exit_code: Number(parsed.exit_code ?? 0),
        success: Boolean(parsed.success ?? parsed.exit_code === 0),
      };
    }
  } catch {
    // Not JSON — fall through
  }
  return null;
}

interface BashToolResultDetailsProps {
  fullText: string;
  containerClassName?: string;
  stdoutClassName?: string;
  stderrClassName?: string;
  showExitCode?: boolean;
  exitCodeClassName?: string;
}

export function BashToolResultDetails({
  fullText,
  containerClassName,
  stdoutClassName,
  stderrClassName,
  showExitCode = true,
  exitCodeClassName,
}: BashToolResultDetailsProps) {
  const bashOutput = parseBashOutput(fullText);

  if (!bashOutput) {
    return (
      <pre
        className={cn(
          "max-h-60 overflow-x-auto whitespace-pre-wrap break-all rounded bg-muted/20 p-1.5 text-[10px] leading-tight",
          containerClassName,
        )}
      >
        {fullText}
      </pre>
    );
  }

  return (
    <div
      className={cn(
        "space-y-0 rounded bg-muted/20 p-1.5 text-[10px] leading-tight",
        containerClassName,
      )}
    >
      {bashOutput.stdout && (
        <pre
          className={cn(
            "max-h-60 overflow-x-auto whitespace-pre-wrap break-all text-inherit",
            stdoutClassName,
          )}
        >
          {bashOutput.stdout}
        </pre>
      )}
      {bashOutput.stderr && (
        <pre
          className={cn(
            "max-h-40 overflow-x-auto whitespace-pre-wrap break-all text-red-600/80 dark:text-red-400/80",
            bashOutput.stdout && "mt-3 border-t border-red-400/20 pt-3",
            stderrClassName,
          )}
        >
          {bashOutput.stderr}
        </pre>
      )}
      {showExitCode && bashOutput.exit_code !== 0 && (
        <div className={cn("mt-3 text-[10px] text-red-500/70", exitCodeClassName)}>
          exit code {bashOutput.exit_code}
        </div>
      )}
    </div>
  );
}

/**
 * Render a bash tool call in Claude Code style.
 *
 * Shows: `$ command` with status icon, collapsed output toggle.
 * Output separates stdout/stderr with visual distinction.
 */
export function BashToolCallCard({ toolCall, toolResult }: BashToolCallCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  const command = toolCall.arguments.command as string | undefined;
  const description = toolCall.arguments.description as string | undefined;
  const isComplete = !!toolResult;
  const hasError = toolResult?.error !== undefined && toolResult?.error !== null;

  // Parse structured bash output
  const fullText = toolResult?.result ? getFullText(toolResult.result) : "";
  const bashOutput = fullText ? parseBashOutput(fullText) : null;

  // Determine if there's meaningful output to show
  const hasStdout = !!bashOutput?.stdout;
  const hasStderr = !!bashOutput?.stderr;
  const hasOutput = bashOutput
    ? hasStdout || hasStderr || bashOutput.exit_code !== 0
    : fullText.length > 0;
  const exitedWithError = bashOutput ? !bashOutput.success : hasError;
  const exitCodeLabel =
    bashOutput && bashOutput.exit_code !== 0 ? `exit ${bashOutput.exit_code}` : null;

  // Status icon
  const statusIcon = isComplete ? (
    exitedWithError ? (
      <span className="text-red-500 text-xs font-bold">!</span>
    ) : (
      <Check className="h-3 w-3 text-green-600/80" />
    )
  ) : (
    <Loader2 className="h-3 w-3 animate-spin text-muted-foreground/60" />
  );

  // Duration display
  const durationLabel = toolResult?.duration_ms
    ? toolResult.duration_ms < 1000
      ? `${toolResult.duration_ms}ms`
      : `${(toolResult.duration_ms / 1000).toFixed(1)}s`
    : null;

  return (
    <div className="text-xs text-muted-foreground/70">
      {/* Command line: status + $ command */}
      <div className="flex items-center gap-1.5">
        {statusIcon}
        <Terminal className="h-3 w-3 opacity-50" />
        <button
          onClick={() => hasOutput && setIsExpanded(!isExpanded)}
          className={`flex items-center gap-1 min-w-0 ${hasOutput ? "cursor-pointer hover:text-muted-foreground" : "cursor-default"}`}
        >
          <span className="inline-flex min-w-0 items-baseline gap-2 font-mono">
            <span className="truncate">
              <span className="opacity-50">$ </span>
              {command ?? "bash"}
            </span>
            {exitCodeLabel && (
              <span className="flex-shrink-0 text-[10px] leading-none text-red-500/70">
                {exitCodeLabel}
              </span>
            )}
          </span>
        </button>
        {durationLabel && (
          <span className="text-[10px] opacity-40 flex-shrink-0">{durationLabel}</span>
        )}
        {hasOutput && (
          <button
            onClick={() => setIsExpanded(!isExpanded)}
            className="opacity-40 hover:opacity-70 flex-shrink-0"
          >
            {isExpanded ? (
              <ChevronDown className="h-3 w-3" />
            ) : (
              <ChevronRight className="h-3 w-3" />
            )}
          </button>
        )}
      </div>

      {/* Description (if present, shown as subtitle) */}
      {description && (
        <div className="ml-[22px] text-[10px] opacity-40 truncate">{description}</div>
      )}

      {/* Tool-level error (not bash stderr) */}
      {hasError && !bashOutput && (
        <div className="text-red-600 ml-[22px] mt-0.5 text-[10px]">Error: {toolResult?.error}</div>
      )}

      {/* Expanded output */}
      {isExpanded && (
        <div className="mt-1 ml-[22px] space-y-2">
          <BashToolResultDetails fullText={fullText} showExitCode={false} />
        </div>
      )}
    </div>
  );
}

/**
 * Check if a tool name is the bash/shell tool.
 */
export function isBashTool(toolName: string): boolean {
  return toolName === "bash" || toolName === "shell" || toolName === "execute_bash";
}
