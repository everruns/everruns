"use client";

// Read-only JSON block for the Trace inspector: pretty-printed, line numbers,
// light token colours, bounded height. Payloads arrive already cut to the
// inline limit by the server, so rendering stays cheap.
//
// Decision: a string value with line breaks (file contents, shell output) is
// shown as the text it holds, one row per line, instead of one escaped line
// full of `\n`. Copy buttons still hand out the exact JSON.

import { useMemo } from "react";
import { cn } from "@/lib/utils";

/** Hard cap on rendered lines; the rest is reachable through copy. */
const MAX_LINES = 2000;

const TOKEN =
  /("(?:[^"\\]|\\.)*")(\s*:)?|\b(true|false|null)\b|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g;

function highlight(line: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  let last = 0;
  for (const match of line.matchAll(TOKEN)) {
    const index = match.index ?? 0;
    if (index > last) out.push(line.slice(last, index));
    const [text, str, colon, literal, number] = match;
    if (str !== undefined && colon !== undefined) {
      out.push(
        <span key={index} className="font-medium text-primary dark:text-accent">
          {str}
        </span>,
        colon,
      );
    } else if (str !== undefined) {
      out.push(
        <span key={index} className="text-success">
          {str}
        </span>,
      );
    } else if (literal !== undefined || number !== undefined) {
      out.push(
        <span key={index} className="text-info">
          {text}
        </span>,
      );
    } else {
      out.push(text);
    }
    last = index + text.length;
  }
  if (last < line.length) out.push(line.slice(last));
  return out;
}

/** A string on one pretty-printed line: indent, optional key, the string, a comma. */
const STRING_LINE = /^(\s*)("(?:[^"\\]|\\.)*":\s*)?"((?:[^"\\]|\\.)*)"(,?)$/;

type Line = { text: string; block?: boolean };

/** Pretty-print, expanding multi-line strings into rows of their text. */
export function toLines(source: string): Line[] {
  const out: Line[] = [];
  for (const line of source.split("\n")) {
    const match = STRING_LINE.exec(line);
    if (!match || !match[3].includes("\\n")) {
      out.push({ text: line });
      continue;
    }
    // The trailing comma is dropped: the rows read as text, not as JSON.
    const [, indent, key, raw] = match;
    let decoded: string;
    try {
      decoded = JSON.parse(`"${raw}"`) as string;
    } catch {
      out.push({ text: line });
      continue;
    }
    if (key) out.push({ text: `${indent}${key.trimEnd()}` });
    for (const textLine of decoded.split("\n")) {
      out.push({ text: `${indent}  ${textLine}`, block: true });
    }
  }
  return out;
}

export function JsonView({
  value,
  text,
  error,
  className,
}: {
  value?: unknown;
  /** Raw text instead of a value, e.g. a cut preview. */
  text?: string;
  error?: boolean;
  className?: string;
}) {
  const lines = useMemo<Line[]>(() => {
    if (text !== undefined || typeof value === "string") {
      return (text ?? (value as string))
        .split("\n")
        .slice(0, MAX_LINES)
        .map((line) => ({ text: line }));
    }
    return toLines(JSON.stringify(value ?? null, null, 2)).slice(0, MAX_LINES);
  }, [value, text]);
  return (
    <div
      className={cn(
        "max-h-[340px] overflow-auto border bg-muted/60 font-mono text-xs leading-[1.6]",
        error && "border-destructive/50",
        className,
      )}
    >
      <table className="w-full border-collapse">
        <tbody>
          {lines.map((line, i) => (
            <tr key={i}>
              <td className="w-8 pr-2 text-right align-top text-muted-foreground/60 select-none">
                {i + 1}
              </td>
              <td
                className={cn(
                  "pr-3 break-all whitespace-pre-wrap",
                  line.block && "border-l-2 border-success/40 pl-1.5 text-foreground",
                )}
              >
                {line.block ? line.text : highlight(line.text)}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
