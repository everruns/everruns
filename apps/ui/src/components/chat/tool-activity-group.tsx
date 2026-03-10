"use client";

/**
 * Decisions:
 * - Group tool-only assistant steps into a single activity block so the transcript reads like progress, not raw logs.
 * - Shell calls stay standalone in transcript order; they should not be wrapped in the grouped activity card.
 * - Error emphasis relies on iconography and copy, not red box styling around the activity row.
 * - Keep motion CSS-only: fade/slide in, status transitions, and collapsible output without adding another runtime dependency.
 * - Match the Slate system: sharp corners, grayscale surfaces, and gold border accents for active work.
 */

import { useMemo, useState } from "react";
import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronRight,
  Loader2,
  MonitorSmartphone,
} from "lucide-react";
import type { ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { BashToolCallCard, isBashTool, parseBashOutput } from "./bash-tool-call-card";
import { ReadFileToolCallCard, isReadFileTool } from "./read-file-tool-call-card";
import { getFullText, type ToolCallContent } from "./tool-call-utils";
import { TodoListRenderer, isWriteTodosTool } from "./todo-list-renderer";
import { WriteFileToolCallCard, isWriteLikeTool } from "./write-file-tool-call-card";

interface ToolActivityGroupProps {
  toolCalls: ToolCallContent[];
  toolResultsMap: Map<string, ToolCompletedData>;
  mode?: "server" | "client";
}

type ActivitySegment =
  | { type: "group"; toolCalls: ToolCallContent[] }
  | { type: "shell"; toolCall: ToolCallContent }
  | { type: "read_file"; toolCall: ToolCallContent }
  | { type: "write_file"; toolCall: ToolCallContent };

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

  if (isReadFileTool(name)) {
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

  if (name === "secret_store") {
    const operation = args.operation;
    const secretName = args.name;
    if (operation === "list") return "List secrets";
    if (
      typeof operation === "string" &&
      typeof secretName === "string" &&
      secretName.trim().length > 0
    ) {
      return `${toTitleCase(operation)} ${secretName}`;
    }
    return "Secret Store";
  }

  if (name === "kv_store") {
    const operation = args.operation;
    const key = args.key;
    if (operation === "list") return "List stored values";
    if (typeof operation === "string" && typeof key === "string" && key.trim().length > 0) {
      return `${toTitleCase(operation)} ${key}`;
    }
    return "Key Value Store";
  }

  return toolCall.display_name ?? toTitleCase(name);
}

function summarizeToolCalls(toolCalls: ToolCallContent[]): string {
  if (toolCalls.length === 0) return "Working";

  if (toolCalls.length === 1) {
    return getToolLabel(toolCalls[0]);
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

function parseStructuredText(text: string): unknown | null {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function maskSensitiveFields(toolName: string, value: unknown): unknown {
  if (
    toolName !== "secret_store" ||
    typeof value !== "object" ||
    value === null ||
    Array.isArray(value)
  ) {
    return value;
  }

  const record = value as Record<string, unknown>;
  if (!("value" in record) || record.value == null) {
    return value;
  }

  return {
    ...record,
    value: "[hidden]",
  };
}

function summarizeStructuredResult(toolCall: ToolCallContent, parsed: unknown): string | null {
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return null;
  }

  const record = parsed as Record<string, unknown>;

  if (toolCall.name === "secret_store") {
    const operation = record.operation;
    const name = record.name;
    if (operation === "get" && typeof name === "string") {
      return record.found ? `${name} found` : `${name} not found`;
    }
    if (operation === "set" && typeof name === "string") {
      return `${name} saved`;
    }
    if (operation === "delete" && typeof name === "string") {
      return record.deleted ? `${name} deleted` : `${name} not found`;
    }
    if (operation === "list" && typeof record.count === "number") {
      return `${record.count} secret${record.count === 1 ? "" : "s"}`;
    }
  }

  if (toolCall.name === "kv_store") {
    const operation = record.operation;
    const key = record.key;
    if (operation === "get" && typeof key === "string") {
      return record.found ? `${key} found` : `${key} not found`;
    }
    if ((operation === "set" || operation === "delete") && typeof key === "string") {
      return `${key} ${operation === "set" ? "saved" : "deleted"}`;
    }
    if (operation === "list" && typeof record.count === "number") {
      return `${record.count} value${record.count === 1 ? "" : "s"}`;
    }
  }

  if (typeof record.message === "string" && record.message.trim().length > 0) {
    return record.message;
  }

  const scalarEntries = Object.entries(record).filter(
    ([, value]) => ["string", "number", "boolean"].includes(typeof value) || value === null,
  );
  if (scalarEntries.length === 0) return null;

  const preview = scalarEntries
    .slice(0, 2)
    .map(([key, value]) => `${key}: ${value === null ? "null" : String(value)}`)
    .join(" · ");

  return preview.length > 120 ? `${preview.slice(0, 120)}...` : preview;
}

function formatResultDetails(toolCall: ToolCallContent, fullText: string): string {
  const parsed = parseStructuredText(fullText);
  if (!parsed) return fullText;

  return JSON.stringify(maskSensitiveFields(toolCall.name, parsed), null, 2);
}

function getResultPreview(
  toolCall: ToolCallContent,
  result: ToolCompletedData | undefined,
): string | null {
  const fullText = getFullText(result?.result);
  if (!fullText) return null;

  const bashOutput = parseBashOutput(fullText);
  if (bashOutput) {
    const previewSource = bashOutput.stdout || bashOutput.stderr;
    const previewLine = previewSource
      .split("\n")
      .map((line) => line.trim())
      .find((line) => line.length > 0);

    if (previewLine) {
      return previewLine.length > 120 ? `${previewLine.slice(0, 120)}...` : previewLine;
    }

    if (bashOutput.exit_code !== 0) {
      return `exit code ${bashOutput.exit_code}`;
    }

    return null;
  }

  const parsed = parseStructuredText(fullText);
  const structuredPreview = summarizeStructuredResult(toolCall, parsed);
  if (structuredPreview) return structuredPreview;

  const previewLine = fullText
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);

  if (!previewLine) return null;
  return previewLine.length > 120 ? `${previewLine.slice(0, 120)}...` : previewLine;
}

function buildActivitySegments(
  toolCalls: ToolCallContent[],
  mode: "server" | "client",
): ActivitySegment[] {
  if (toolCalls.length === 0) return [];

  if (mode === "client") {
    return [{ type: "group", toolCalls }];
  }

  const segments: ActivitySegment[] = [];
  let currentGroup: ToolCallContent[] = [];

  const pushGroup = () => {
    if (currentGroup.length === 0) return;
    segments.push({ type: "group", toolCalls: currentGroup });
    currentGroup = [];
  };

  for (const toolCall of toolCalls) {
    if (isBashTool(toolCall.name)) {
      pushGroup();
      segments.push({ type: "shell", toolCall });
      continue;
    }

    if (isReadFileTool(toolCall.name)) {
      pushGroup();
      segments.push({ type: "read_file", toolCall });
      continue;
    }

    if (isWriteLikeTool(toolCall.name)) {
      pushGroup();
      segments.push({ type: "write_file", toolCall });
      continue;
    }

    currentGroup.push(toolCall);
  }

  pushGroup();
  return segments;
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
  const hasToolError = !!toolResult?.error;
  const hasError = hasToolError;
  const isComplete = !!toolResult;
  const isRunning = !isComplete && !hasError;

  return (
    <div
      className={cn(
        "animate-tool-row-in py-2 transition-all duration-300",
        isRunning && "border-l-2 border-l-accent bg-[hsl(var(--accent)/0.06)] pl-2",
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
            <span className="truncate text-sm text-foreground">{getToolLabel(toolCall)}</span>
            {isRunning && (
              <span className="animate-tool-pulse text-[10px] uppercase tracking-[0.18em] text-accent-foreground/70">
                {mode === "client" ? "waiting" : "running"}
              </span>
            )}
          </div>

          {hasToolError && (
            <div className="mt-1 text-xs text-red-600 dark:text-red-400">{toolResult?.error}</div>
          )}

          {!hasError && !isExpanded && hasOutput && (
            <div className="mt-1 truncate text-xs text-muted-foreground/70">
              {getResultPreview(toolCall, toolResult)}
            </div>
          )}

          {hasOutput && (
            <button
              type="button"
              onClick={() => setIsExpanded((current) => !current)}
              className="mt-1 inline-flex items-center gap-1 text-[10px] uppercase tracking-[0.18em] text-muted-foreground/65 transition-colors hover:text-foreground"
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

function GroupedActivityCard({
  toolCalls,
  toolResultsMap,
  mode = "server",
}: ToolActivityGroupProps) {
  if (toolCalls.length === 1) {
    const toolCall = toolCalls[0];
    return (
      <ToolActivityRow
        toolCall={toolCall}
        toolResult={toolResultsMap.get(toolCall.id)}
        mode={mode}
      />
    );
  }

  const [isExpanded, setIsExpanded] = useState(true);
  const activityCompletedCount = useMemo(
    () => toolCalls.filter((toolCall) => toolResultsMap.has(toolCall.id)).length,
    [toolCalls, toolResultsMap],
  );
  const isActive = activityCompletedCount < toolCalls.length;

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-3 py-1">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <div className="text-[11px] uppercase tracking-[0.22em] text-muted-foreground/70">
              {summarizeToolCalls(toolCalls)}
            </div>
            <div className="text-[10px] text-muted-foreground/50">
              {activityCompletedCount}/{toolCalls.length}
            </div>
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
          <div className="space-y-1 border-l border-border/60 pl-3">
            {toolCalls.map((toolCall) => (
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
  );
}

export function ToolActivityGroup({
  toolCalls,
  toolResultsMap,
  mode = "server",
}: ToolActivityGroupProps) {
  const todoToolCalls = toolCalls.filter((toolCall) => isWriteTodosTool(toolCall.name));
  const activityToolCalls = toolCalls.filter((toolCall) => !isWriteTodosTool(toolCall.name));
  const activitySegments = useMemo(
    () => buildActivitySegments(activityToolCalls, mode),
    [activityToolCalls, mode],
  );

  if (toolCalls.length === 0) return null;

  return (
    <div className="space-y-3">
      {activitySegments.map((segment, index) => {
        if (segment.type === "shell") {
          return (
            <BashToolCallCard
              key={segment.toolCall.id}
              toolCall={segment.toolCall}
              toolResult={toolResultsMap.get(segment.toolCall.id)}
            />
          );
        }

        if (segment.type === "read_file") {
          return (
            <ReadFileToolCallCard
              key={segment.toolCall.id}
              toolCall={segment.toolCall}
              toolResult={toolResultsMap.get(segment.toolCall.id)}
            />
          );
        }

        if (segment.type === "write_file") {
          return (
            <WriteFileToolCallCard
              key={segment.toolCall.id}
              toolCall={segment.toolCall}
              toolResult={toolResultsMap.get(segment.toolCall.id)}
            />
          );
        }

        return (
          <GroupedActivityCard
            key={`${segment.toolCalls[0]?.id ?? "group"}-${index}`}
            toolCalls={segment.toolCalls}
            toolResultsMap={toolResultsMap}
            mode={mode}
          />
        );
      })}

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
