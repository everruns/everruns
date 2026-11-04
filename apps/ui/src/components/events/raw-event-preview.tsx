"use client";

import { memo, useMemo } from "react";

const MAX_CHARACTERS = 8192;
const MAX_NODES = 128;
const MAX_STRING = 512;
const MAX_DEPTH = 8;

// A scrollbox's height does not bound text layout or serialization work. Walk
// only a small preview before stringifying; copying still uses the original.
export function formatEventPreview(event: unknown): { text: string; truncated: boolean } {
  let remaining = MAX_NODES;
  let truncated = false;
  const shorten = (text: string) => {
    if (text.length <= MAX_STRING) return text;
    truncated = true;
    return `${text.slice(0, MAX_STRING)}…`;
  };
  const visit = (value: unknown, depth: number): unknown => {
    if (remaining-- <= 0 || depth >= MAX_DEPTH) {
      truncated = true;
      return "…";
    }
    if (typeof value === "string") return shorten(value);
    if (value === null || typeof value !== "object") return value;
    if (Array.isArray(value)) {
      const result = [];
      for (const item of value) {
        if (remaining <= 0 || result.length >= 32) {
          truncated = true;
          break;
        }
        result.push(visit(item, depth + 1));
      }
      return result;
    }
    const result: Record<string, unknown> = Object.create(null);
    let count = 0;
    for (const key in value) {
      if (!Object.hasOwn(value, key)) continue;
      if (remaining <= 0 || count++ >= 32) {
        truncated = true;
        break;
      }
      result[shorten(key)] = visit((value as Record<string, unknown>)[key], depth + 1);
    }
    return result;
  };
  const formatted = JSON.stringify(visit(event, 0), null, 2) ?? "null";
  if (formatted.length > MAX_CHARACTERS) truncated = true;
  return { text: formatted.slice(0, MAX_CHARACTERS), truncated };
}

export const RawEventPreview = memo(function RawEventPreview({ event }: { event: unknown }) {
  const preview = useMemo(() => formatEventPreview(event), [event]);
  return (
    <>
      <pre className="whitespace-pre-wrap break-all text-xs bg-muted p-2 rounded max-h-[200px] overflow-y-auto">
        {preview.text}
      </pre>
      {preview.truncated && (
        <p className="mt-1 text-xs text-muted-foreground">
          Preview shortened. Copy includes the complete event.
        </p>
      )}
    </>
  );
});
