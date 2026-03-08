"use client";

/**
 * Decisions:
 * - Group tool-only assistant steps into a single activity block so the transcript reads like progress, not raw logs.
 * - Keep motion CSS-only: fade/slide in, status transitions, and collapsible output without adding another runtime dependency.
 * - Match the Slate system: sharp corners, grayscale surfaces, and gold border accents for active work.
 */

import { useMemo, useState } from "react";
import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronRight,
  Info,
  Loader2,
  MonitorSmartphone,
  Terminal,
} from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { isBashTool } from "./bash-tool-call-card";
import { getFullText, type ToolCallContent } from "./tool-call-utils";
import { TodoListRenderer, isWriteTodosTool } from "./todo-list-renderer";

interface ToolActivityGroupProps {
  toolCalls: ToolCallContent[];
  toolResultsMap: Map<string, ToolCompletedData>;
  mode?: "server" | "client";
}

type ToolCategory = "read" | "search" | "write" | "shell" | "tool";

function pluralize(count: number, singular: string, plural: string) {
  return `${count} ${count === 1 ? singular : plural}`;
}

function toTitleCase(value: string): string {
  return value
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function getCategory(name: string): ToolCategory {
  if (isBashTool(name)) return "shell";
  if (
    [
      "list_files",
      "read_file",
      "read_many_files",
      "session_read_file",
      "list_capabilities",
    ].includes(name)
  ) {
    return "read";
  }
  if (
    name === "search" ||
    name === "search_web" ||
    name === "grep_files" ||
    name.endsWith("__search")
  ) {
    return "search";
  }
  if (
    [
      "write_file",
      "edit_file",
      "replace_in_file",
      "append_file",
      "move_file",
      "delete_file",
      "mkdir",
    ].includes(name)
  ) {
    return "write";
  }
  return "tool";
}

function formatLocation(value: unknown): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    return "current directory";
  }
  if (value === "." || value === "/workspace") {
    return "current directory";
  }
  return value;
}

function basename(value: string): string {
  const clean = value.replace(/\/+$/, "");
  const parts = clean.split("/");
  return parts[parts.length - 1] || clean;
}

function getToolLabel(toolCall: ToolCallContent): string {
  const { arguments: args, name } = toolCall;

  if (isBashTool(name)) {
    const command = args.command;
    return typeof command === "string" && command.trim().length > 0 ? `$ ${command}` : "Shell";
  }

  if (name === "list_files") {
    return `List files in ${formatLocation(args.path)}`;
  }

  if (name === "read_file") {
    const path = args.path;
    return typeof path === "string" && path.trim().length > 0
      ? `Read ${basename(path)}`
      : "Read file";
  }

  if (name === "grep_files") {
    const pattern = args.pattern;
    return typeof pattern === "string" && pattern.trim().length > 0
      ? `Find ${pattern}`
      : "Search files";
  }

  if (name === "search_web" || name === "search" || name.endsWith("__search")) {
    const query = args.query ?? args.search ?? args.q;
    return typeof query === "string" && query.trim().length > 0
      ? `Search web for ${query}`
      : "Search web";
  }

  if (name === "write_file") {
    const path = args.path;
    return typeof path === "string" && path.trim().length > 0
      ? `Write ${basename(path)}`
      : "Write file";
  }

  if (name === "replace_in_file" || name === "edit_file") {
    const path = args.path;
    return typeof path === "string" && path.trim().length > 0
      ? `Edit ${basename(path)}`
      : "Edit file";
  }

  return toolCall.display_name ?? toTitleCase(name);
}

function summarizeToolCalls(toolCalls: ToolCallContent[]): string {
  if (toolCalls.length === 0) return "Working";

  if (toolCalls.length === 1) {
    const [toolCall] = toolCalls;
    if (isBashTool(toolCall.name)) return "Shell";
    return getToolLabel(toolCall);
  }

  const counts = {
    read: 0,
    search: 0,
    write: 0,
    shell: 0,
    tool: 0,
  };

  for (const toolCall of toolCalls) {
    counts[getCategory(toolCall.name)] += 1;
  }

  const parts: string[] = [];
  if (counts.read > 0) parts.push(pluralize(counts.read, "read", "reads"));
  if (counts.search > 0) parts.push(pluralize(counts.search, "search", "searches"));
  if (counts.write > 0) parts.push(pluralize(counts.write, "write", "writes"));
  if (counts.shell > 0) parts.push(pluralize(counts.shell, "shell", "shells"));
  if (counts.tool > 0) parts.push(pluralize(counts.tool, "tool", "tools"));

  return `Exploring ${parts.join(", ")}`;
}

function getResultPreview(result: ToolCompletedData | undefined): string | null {
  const fullText = getFullText(result?.result);
  if (!fullText) return null;

  const previewLine = fullText
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);

  if (!previewLine) return null;
  return previewLine.length > 120 ? `${previewLine.slice(0, 120)}...` : previewLine;
}

function InfoHint({ text }: { text: string }) {
  return (
    <Tooltip>
      <TooltipTrigger
        className="rounded p-0.5 text-muted-foreground/55 transition-colors hover:bg-muted hover:text-foreground"
        aria-label="Tool activity info"
      >
        <Info className="h-3 w-3" />
      </TooltipTrigger>
      <TooltipContent className="max-w-56 text-xs leading-5">{text}</TooltipContent>
    </Tooltip>
  );
}

function ToolActivityRow({
  toolCall,
  toolResult,
  mode,
}: {
  toolCall: ToolCallContent;
  toolResult?: ToolCompletedData;
  mode: "server" | "client";
}) {
  const [isExpanded, setIsExpanded] = useState(false);
  const fullText = getFullText(toolResult?.result);
  const hasOutput = fullText.length > 0;
  const hasError = !!toolResult?.error;
  const isComplete = !!toolResult;
  const isRunning = !isComplete && !hasError;

  return (
    <div
      className={cn(
        "animate-tool-row-in border px-3 py-2.5 transition-all duration-300",
        isComplete && !hasError && "border-transparent bg-transparent",
        isRunning && "border-border/70 border-l-2 border-l-accent bg-[hsl(var(--accent)/0.06)]",
        hasError && "border-red-200 bg-red-500/[0.04] dark:border-red-900/70 dark:bg-red-950/20",
      )}
    >
      <div className="flex items-start gap-2">
        <div className="mt-0.5 flex h-4 w-4 items-center justify-center">
          {hasError ? (
            <AlertCircle className="h-3.5 w-3.5 text-red-500" />
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
            {isBashTool(toolCall.name) && mode === "server" && (
              <Terminal className="mt-0.5 h-3.5 w-3.5 flex-shrink-0 text-primary/55" />
            )}
            <span className="truncate text-sm text-foreground">{getToolLabel(toolCall)}</span>
            {isRunning && (
              <span className="animate-tool-pulse text-[10px] uppercase tracking-[0.18em] text-accent-foreground/70">
                {mode === "client" ? "waiting" : "running"}
              </span>
            )}
          </div>

          {hasError && (
            <div className="mt-1 text-xs text-red-600 dark:text-red-400">{toolResult?.error}</div>
          )}

          {!hasError && !isExpanded && hasOutput && (
            <div className="mt-1 truncate text-xs text-muted-foreground/70">
              {getResultPreview(toolResult)}
            </div>
          )}

          {hasOutput && (
            <button
              type="button"
              onClick={() => setIsExpanded((current) => !current)}
              className="mt-1.5 inline-flex items-center gap-1 text-[10px] uppercase tracking-[0.18em] text-muted-foreground/65 transition-colors hover:text-foreground"
            >
              {isExpanded ? (
                <ChevronDown className="h-3 w-3" />
              ) : (
                <ChevronRight className="h-3 w-3" />
              )}
              {isExpanded ? "hide details" : "details"}
            </button>
          )}

          <div
            className={cn(
              "grid transition-all duration-300 ease-out",
              isExpanded ? "mt-2 grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
            )}
          >
            <div className="min-h-0 overflow-hidden">
              {hasOutput && (
                <pre className="overflow-x-auto border border-border/60 bg-background px-3 py-3 font-mono text-[11px] leading-relaxed text-muted-foreground/85">
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

export function ToolActivityGroup({
  toolCalls,
  toolResultsMap,
  mode = "server",
}: ToolActivityGroupProps) {
  const [isExpanded, setIsExpanded] = useState(true);
  const todoToolCalls = toolCalls.filter((toolCall) => isWriteTodosTool(toolCall.name));
  const activityToolCalls = toolCalls.filter((toolCall) => !isWriteTodosTool(toolCall.name));

  const activityCompletedCount = useMemo(
    () => activityToolCalls.filter((toolCall) => toolResultsMap.has(toolCall.id)).length,
    [activityToolCalls, toolResultsMap],
  );
  const isActive = activityCompletedCount < activityToolCalls.length;

  if (toolCalls.length === 0) return null;

  return (
    <div className="space-y-3">
      {activityToolCalls.length > 0 && (
        <div
          className={cn(
            "overflow-hidden border bg-card/95",
            isActive ? "border-border border-l-2 border-l-accent" : "border-border",
          )}
        >
          <div className="flex items-center justify-between gap-3 px-4 py-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <div className="text-sm text-foreground">
                  {summarizeToolCalls(activityToolCalls)}
                </div>
                <InfoHint
                  text={
                    mode === "client"
                      ? "Client-side tools wait for a browser action before the assistant continues."
                      : "Grouped tool execution keeps intermediate reads, searches, and edits compact inside the transcript."
                  }
                />
              </div>
              <div className="mt-0.5 text-xs text-muted-foreground/70">
                {activityCompletedCount} of {activityToolCalls.length} complete
              </div>
            </div>
            <div className="flex items-center gap-2">
              {isActive ? (
                <div className="flex items-center gap-2 text-[10px] uppercase tracking-[0.22em] text-accent-foreground/70">
                  <span className="animate-tool-pulse inline-flex h-1.5 w-1.5 bg-accent" />
                </div>
              ) : null}
              <button
                type="button"
                onClick={() => setIsExpanded((current) => !current)}
                className="rounded p-1 text-muted-foreground/60 transition-colors hover:bg-muted hover:text-foreground"
                aria-label={isExpanded ? "Collapse tool activity" : "Expand tool activity"}
              >
                {isExpanded ? (
                  <ChevronDown className="h-4 w-4" />
                ) : (
                  <ChevronRight className="h-4 w-4" />
                )}
              </button>
            </div>
          </div>

          <div
            className={cn(
              "grid transition-all duration-300 ease-out",
              isExpanded ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
            )}
          >
            <div className="min-h-0 overflow-hidden">
              <div className="space-y-1 px-3 py-2">
                {activityToolCalls.map((toolCall) => (
                  <ToolActivityRow
                    key={toolCall.id}
                    toolCall={toolCall}
                    toolResult={toolResultsMap.get(toolCall.id)}
                    mode={mode}
                  />
                ))}
              </div>
            </div>
          </div>
        </div>
      )}

      {todoToolCalls.map((toolCall) => {
        const toolResult = toolResultsMap.get(toolCall.id);
        return (
          <TodoListRenderer
            key={toolCall.id}
            arguments={toolCall.arguments}
            result={toolResult?.result}
            isExecuting={!toolResult}
            error={toolResult?.error}
          />
        );
      })}
    </div>
  );
}
