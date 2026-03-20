"use client";

/**
 * Decisions:
 * - `read_file` should read like shell output: one inline row, no outer activity card.
 * - The tool emits structured JSON in the text part; parse it and render the actual file body.
 * - Native image content parts are first-class output and must render inline instead of being dropped.
 */

import { useMemo, useState } from "react";
import { Check, ChevronDown, ChevronRight, FileText, Loader2 } from "lucide-react";
import type { ContentPart, ToolCompletedData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { basename } from "@/lib/path-utils";
import { formatFileSize } from "@/lib/formatting";
import { getFullText, type ToolCallContent } from "./tool-call-utils";

interface ReadFileToolCallCardProps {
  toolCall: ToolCallContent;
  toolResult?: ToolCompletedData;
}

interface ReadFilePayload {
  path?: string;
  content?: string | null;
  encoding?: string;
  size_bytes?: number;
  media_type?: string;
}

interface ParsedReadFileImage {
  src: string;
  mediaType?: string;
}

interface ParsedReadFileResult {
  path?: string;
  content: string | null;
  encoding?: string;
  sizeBytes?: number;
  images: ParsedReadFileImage[];
}

function getPathFromArguments(toolCall: ToolCallContent): string | undefined {
  const path = toolCall.arguments.path;
  return typeof path === "string" && path.trim().length > 0 ? path : undefined;
}

function isImagePart(
  part: ContentPart,
): part is { type: "image"; url?: string; base64?: string; media_type?: string } {
  return part.type === "image";
}

function buildImageSrc(part: {
  url?: string;
  base64?: string;
  media_type?: string;
}): string | null {
  if (typeof part.url === "string" && part.url.length > 0) {
    return part.url;
  }
  if (typeof part.base64 === "string" && part.base64.length > 0) {
    const mediaType =
      typeof part.media_type === "string" && part.media_type.length > 0
        ? part.media_type
        : "image/png";
    return `data:${mediaType};base64,${part.base64}`;
  }
  return null;
}

function parseReadFilePayload(text: string): ReadFilePayload | null {
  if (!text) return null;

  try {
    const parsed = JSON.parse(text);
    if (typeof parsed === "object" && parsed !== null) {
      return parsed as ReadFilePayload;
    }
  } catch {
    // Fall back to raw text for older or malformed payloads.
  }

  return null;
}

function parseReadFileResult(
  toolCall: ToolCallContent,
  result: ContentPart[] | undefined,
): ParsedReadFileResult {
  const rawText = getFullText(result);
  const payload = parseReadFilePayload(rawText);
  const images = (result ?? []).filter(isImagePart).reduce<ParsedReadFileImage[]>((all, part) => {
    const src = buildImageSrc(part);
    if (!src) return all;
    all.push({
      src,
      mediaType: part.media_type,
    });
    return all;
  }, []);

  return {
    path:
      (typeof payload?.path === "string" && payload.path.length > 0 ? payload.path : undefined) ??
      getPathFromArguments(toolCall),
    content:
      typeof payload?.content === "string" ? payload.content : payload ? null : (rawText ?? null),
    encoding: typeof payload?.encoding === "string" ? payload.encoding : undefined,
    sizeBytes: typeof payload?.size_bytes === "number" ? payload.size_bytes : undefined,
    images,
  };
}

function getContentPreview(content: string | null): string | null {
  if (!content) return null;

  const firstNonEmptyLine = content
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);

  if (!firstNonEmptyLine) return null;
  return firstNonEmptyLine.length > 120
    ? `${firstNonEmptyLine.slice(0, 120)}...`
    : firstNonEmptyLine;
}

export function isReadFileTool(toolName: string): boolean {
  return toolName === "read_file" || toolName === "session_read_file";
}

export function ReadFileToolCallCard({ toolCall, toolResult }: ReadFileToolCallCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);
  const parsed = useMemo(
    () => parseReadFileResult(toolCall, toolResult?.result),
    [toolCall, toolResult?.result],
  );

  const isComplete = !!toolResult;
  const hasError = toolResult?.error !== undefined && toolResult?.error !== null;
  const hasImages = parsed.images.length > 0;
  const hasTextContent = typeof parsed.content === "string" && parsed.content.length > 0;
  const hasOutput = hasImages || hasTextContent;
  const isBinaryWithoutPreview = parsed.encoding === "base64" && !hasImages && !hasTextContent;
  const durationLabel = toolResult?.duration_ms
    ? toolResult.duration_ms < 1000
      ? `${toolResult.duration_ms}ms`
      : `${(toolResult.duration_ms / 1000).toFixed(1)}s`
    : null;
  const preview =
    getContentPreview(parsed.content) ??
    (hasImages ? `${parsed.images.length} image${parsed.images.length === 1 ? "" : "s"}` : null) ??
    (isBinaryWithoutPreview
      ? `binary file${formatFileSize(parsed.sizeBytes) ? ` (${formatFileSize(parsed.sizeBytes)})` : ""}`
      : null);
  const title = parsed.path ? `Read ${basename(parsed.path)}` : "Read file";
  const showDetails = hasImages || isExpanded;

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
        <FileText className="h-3 w-3 opacity-50" />
        <button
          type="button"
          onClick={() => hasTextContent && setIsExpanded((current) => !current)}
          className={cn(
            "flex min-w-0 items-center gap-1",
            hasTextContent ? "cursor-pointer hover:text-muted-foreground" : "cursor-default",
          )}
        >
          <span className="truncate text-sm text-foreground">{title}</span>
        </button>
        {durationLabel && (
          <span className="flex-shrink-0 text-[10px] opacity-40">{durationLabel}</span>
        )}
        {hasTextContent && (
          <button
            type="button"
            onClick={() => setIsExpanded((current) => !current)}
            className="flex-shrink-0 opacity-40 transition-opacity hover:opacity-70"
            aria-label={isExpanded ? "Collapse file output" : "Expand file output"}
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

      {!hasError && preview && !showDetails && (
        <div className="ml-[22px] mt-0.5 truncate text-[10px] opacity-70">{preview}</div>
      )}

      {showDetails && hasOutput && !hasError && (
        <div className="ml-[22px] mt-1.5 space-y-3">
          {hasTextContent && isExpanded && (
            <pre className="overflow-x-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-muted-foreground/85">
              {parsed.content}
            </pre>
          )}

          {hasImages && (
            <div className="space-y-2">
              {parsed.images.map((image, index) => (
                // eslint-disable-next-line @next/next/no-img-element -- tool output can be data URLs or arbitrary remote images.
                <img
                  key={`${image.src}-${index}`}
                  src={image.src}
                  alt={parsed.path ? basename(parsed.path) : `Read file image ${index + 1}`}
                  className="max-h-80 max-w-full object-contain"
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
