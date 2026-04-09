/**
 * Decisions:
 * - Keep label/summary/result formatting pure so cards and tests can reuse the same behavior.
 * - Segment shell/read/write rows before render so transcript ordering stays deterministic.
 * - Secret tool results always mask secret values, even in expanded details.
 */

import type { ToolCompletedData } from "@/lib/api/types";
import { isRecord } from "@/lib/api/types";
import { basename } from "@/lib/path-utils";
import {
  getToolCategory,
  isBashTool,
  isReadFileTool,
  isWriteLikeTool,
  type ToolCategory,
} from "@/lib/tool-registry";
import { formatMessage, getSupportedLocale, type SupportedLocale } from "@/lib/i18n";
import { parseBashOutput } from "./bash-tool-call-card";
import { getFullText, type ToolCallContent } from "./tool-call-utils";

export type ActivitySegment =
  | { type: "group"; toolCalls: ToolCallContent[] }
  | { type: "shell"; toolCall: ToolCallContent }
  | { type: "read_file"; toolCall: ToolCallContent }
  | { type: "write_file"; toolCall: ToolCallContent };

function toTitleCase(value: string): string {
  return value
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

function uiLocale(locale: string): SupportedLocale {
  return getSupportedLocale(locale);
}

function formatLocation(value: unknown, locale: string): string {
  if (typeof value !== "string" || value.trim().length === 0)
    return formatMessage(uiLocale(locale), "current_directory");
  if (value === "." || value === "/workspace")
    return formatMessage(uiLocale(locale), "current_directory");
  return value;
}

export function getToolLabel(toolCall: ToolCallContent, locale: string): string {
  const currentLocale = uiLocale(locale);
  const { arguments: args, name } = toolCall;

  if (isBashTool(name)) {
    const command = args.command;
    return typeof command === "string" && command.trim().length > 0
      ? `$ ${command}`
      : formatMessage(currentLocale, "shell");
  }

  if (name === "list_files")
    return formatMessage(currentLocale, "list_files_in", {
      value: formatLocation(args.path, locale),
    });

  if (isReadFileTool(name)) {
    const path = args.path;
    return typeof path === "string" && path.trim().length > 0
      ? formatMessage(currentLocale, "read_named_file", { value: basename(path) })
      : formatMessage(currentLocale, "read_file");
  }

  if (name === "grep_files") {
    const pattern = args.pattern;
    return typeof pattern === "string" && pattern.trim().length > 0
      ? formatMessage(currentLocale, "find_value", { value: pattern })
      : formatMessage(currentLocale, "search_files");
  }

  if (name === "search_web" || name === "search" || name.endsWith("__search")) {
    const query = args.query ?? args.search ?? args.q;
    return typeof query === "string" && query.trim().length > 0
      ? formatMessage(currentLocale, "search_web_for", { value: query })
      : formatMessage(currentLocale, "search_web");
  }

  if (name === "write_file") {
    const path = args.path;
    return typeof path === "string" && path.trim().length > 0
      ? formatMessage(currentLocale, "write_named_file", { value: basename(path) })
      : formatMessage(currentLocale, "write_file");
  }

  if (name === "replace_in_file" || name === "edit_file") {
    const path = args.path;
    return typeof path === "string" && path.trim().length > 0
      ? formatMessage(currentLocale, "edit_named_file", { value: basename(path) })
      : formatMessage(currentLocale, "edit_file");
  }

  if (name === "secret_store") {
    const operation = args.operation;
    const secretName = args.name;
    if (operation === "list") return formatMessage(currentLocale, "list_secrets");
    if (
      typeof operation === "string" &&
      typeof secretName === "string" &&
      secretName.trim().length > 0
    ) {
      return `${toTitleCase(operation)} ${secretName}`;
    }
    return formatMessage(currentLocale, "secret_store");
  }

  if (name === "kv_store") {
    const operation = args.operation;
    const key = args.key;
    if (operation === "list") return formatMessage(currentLocale, "list_stored_values");
    if (typeof operation === "string" && typeof key === "string" && key.trim().length > 0) {
      return `${toTitleCase(operation)} ${key}`;
    }
    return formatMessage(currentLocale, "key_value_store");
  }

  return toolCall.display_name ?? toTitleCase(name);
}

export function summarizeToolCalls(toolCalls: ToolCallContent[], locale: string): string {
  const currentLocale = uiLocale(locale);
  if (toolCalls.length === 0) return formatMessage(currentLocale, "working");
  if (toolCalls.length === 1) return getToolLabel(toolCalls[0], locale);

  const counts: Record<ToolCategory, number> = {
    read: 0,
    search: 0,
    write: 0,
    shell: 0,
    todo: 0,
    tool: 0,
  };

  for (const toolCall of toolCalls) {
    counts[getToolCategory(toolCall.name)] += 1;
  }

  const parts: string[] = [];
  if (counts.read > 0)
    parts.push(
      formatMessage(currentLocale, "read_count", {
        count: counts.read,
        label:
          counts.read === 1
            ? formatMessage(currentLocale, "read_label_one")
            : formatMessage(currentLocale, "read_label_many"),
      }),
    );
  if (counts.search > 0)
    parts.push(
      formatMessage(currentLocale, "read_count", {
        count: counts.search,
        label:
          counts.search === 1
            ? formatMessage(currentLocale, "search_label_one")
            : formatMessage(currentLocale, "search_label_many"),
      }),
    );
  if (counts.write > 0)
    parts.push(
      formatMessage(currentLocale, "read_count", {
        count: counts.write,
        label:
          counts.write === 1
            ? formatMessage(currentLocale, "write_label_one")
            : formatMessage(currentLocale, "write_label_many"),
      }),
    );
  if (counts.shell > 0)
    parts.push(
      formatMessage(currentLocale, "read_count", {
        count: counts.shell,
        label:
          counts.shell === 1
            ? formatMessage(currentLocale, "shell_label_one")
            : formatMessage(currentLocale, "shell_label_many"),
      }),
    );
  if (counts.tool > 0)
    parts.push(
      formatMessage(currentLocale, "read_count", {
        count: counts.tool,
        label:
          counts.tool === 1
            ? formatMessage(currentLocale, "tool_label_one")
            : formatMessage(currentLocale, "tool_label_many"),
      }),
    );

  return formatMessage(currentLocale, "exploring", { value: parts.join(", ") });
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
  if (toolName !== "secret_store" || !isRecord(value)) return value;
  if (!("value" in value) || value.value == null) return value;
  return { ...value, value: "[hidden]" };
}

function summarizeStructuredResult(toolCall: ToolCallContent, parsed: unknown): string | null {
  if (!isRecord(parsed)) return null;
  const record = parsed;

  if (toolCall.name === "secret_store") {
    const operation = record.operation;
    const name = record.name;
    if (operation === "get" && typeof name === "string")
      return record.found ? `${name} found` : `${name} not found`;
    if (operation === "set" && typeof name === "string") return `${name} saved`;
    if (operation === "delete" && typeof name === "string")
      return record.deleted ? `${name} deleted` : `${name} not found`;
    if (operation === "list" && typeof record.count === "number")
      return `${record.count} secret${record.count === 1 ? "" : "s"}`;
  }

  if (toolCall.name === "kv_store") {
    const operation = record.operation;
    const key = record.key;
    if (operation === "get" && typeof key === "string")
      return record.found ? `${key} found` : `${key} not found`;
    if ((operation === "set" || operation === "delete") && typeof key === "string") {
      return `${key} ${operation === "set" ? "saved" : "deleted"}`;
    }
    if (operation === "list" && typeof record.count === "number")
      return `${record.count} value${record.count === 1 ? "" : "s"}`;
  }

  if (typeof record.message === "string" && record.message.trim().length > 0) return record.message;

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

/** Fields filtered from human-friendly details view (noise for the user). */
const HIDDEN_DETAIL_FIELDS = new Set([
  "id",
  "_id",
  "agent_id",
  "session_id",
  "harness_id",
  "capability_id",
  "organization_id",
  "created_at",
  "updated_at",
  "deleted_at",
]);

function formatValueForDisplay(value: unknown, indent = 0): string {
  if (value === null || value === undefined) return "null";
  if (typeof value === "boolean") return value ? "yes" : "no";
  if (typeof value === "number") return String(value);
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    if (value.length === 0) return "(none)";
    const items = value.map((item) => {
      if (typeof item === "string") return item;
      return JSON.stringify(item);
    });
    return items.join(", ");
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).filter(
      ([key]) => !HIDDEN_DETAIL_FIELDS.has(key),
    );
    if (entries.length === 0) return "{}";
    const prefix = " ".repeat(indent + 2);
    return entries
      .map(([key, val]) => `${prefix}${key}: ${formatValueForDisplay(val, indent + 2)}`)
      .join("\n");
  }
  return String(value);
}

export function formatResultDetails(toolCall: ToolCallContent, fullText: string): string {
  const parsed = parseStructuredText(fullText);
  if (!parsed) return fullText;

  const masked = maskSensitiveFields(toolCall.name, parsed);
  if (!isRecord(masked)) return fullText;

  const entries = Object.entries(masked).filter(([key]) => !HIDDEN_DETAIL_FIELDS.has(key));
  if (entries.length === 0) return fullText;

  return entries.map(([key, value]) => `${key}: ${formatValueForDisplay(value)}`).join("\n");
}

export function getResultPreview(
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

    if (previewLine)
      return previewLine.length > 120 ? `${previewLine.slice(0, 120)}...` : previewLine;
    if (bashOutput.exit_code !== 0) return `exit code ${bashOutput.exit_code}`;
    return null;
  }

  const structuredPreview = summarizeStructuredResult(toolCall, parseStructuredText(fullText));
  if (structuredPreview) return structuredPreview;

  const previewLine = fullText
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);

  if (!previewLine) return null;
  return previewLine.length > 120 ? `${previewLine.slice(0, 120)}...` : previewLine;
}

export function buildActivitySegments(
  toolCalls: ToolCallContent[],
  mode: "server" | "client",
): ActivitySegment[] {
  if (toolCalls.length === 0) return [];
  if (mode === "client") return [{ type: "group", toolCalls }];

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
