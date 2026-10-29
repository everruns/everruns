/**
 * A2UI utilities for detecting and splitting A2UI JSON code blocks from
 * markdown text in chat messages.
 *
 * A2UI JSON is wrapped in ```a2ui fenced code blocks by the LLM when the
 * `a2ui` capability is enabled. This module splits message text into
 * alternating markdown, openui, and a2ui segments so the renderer can
 * dispatch each to its own component.
 *
 * @see knowledge/ui/a2ui.md
 */

export type A2UISegmentKind = "markdown" | "openui" | "a2ui";

export interface A2UIMessageSegment {
  type: A2UISegmentKind;
  content: string;
}

/**
 * Regex matching ```a2ui ... ``` and ```openui ... ``` fenced code blocks.
 * Captures the fence kind in group 1 and the content in group 2.
 * Unclosed blocks at end-of-string are captured so streaming output renders
 * progressively.
 */
const BLOCK_RE = /```(a2ui|openui)\s*\n([\s\S]*?)(?:```|$)/g;

/**
 * Split message text into alternating markdown / openui / a2ui segments.
 *
 * Fast path: if no fenced blocks are present, the whole text is returned as
 * a single markdown segment.
 */
export function splitA2UIBlocks(text: string): A2UIMessageSegment[] {
  const segments: A2UIMessageSegment[] = [];
  let lastIndex = 0;

  BLOCK_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = BLOCK_RE.exec(text)) !== null) {
    if (match.index > lastIndex) {
      const md = text.slice(lastIndex, match.index);
      if (md.trim()) segments.push({ type: "markdown", content: md });
    }

    const kind = match[1] as "a2ui" | "openui";
    const content = match[2];
    if (content.trim()) segments.push({ type: kind, content });

    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < text.length) {
    const trailing = text.slice(lastIndex);
    if (trailing.trim()) segments.push({ type: "markdown", content: trailing });
  }

  if (segments.length === 0 && text.trim()) {
    segments.push({ type: "markdown", content: text });
  }

  return segments;
}

/** True if `text` contains any generative-UI fenced code blocks. */
export function hasA2UIBlocks(text: string): boolean {
  return /^(?:`{3,}|~{3,})(?:a2ui|openui)(?:\s|$)/m.test(text);
}

/** True if `text` contains at least one ```a2ui fence and no ```openui fences. */
export function hasOnlyA2UIBlocks(text: string): boolean {
  return /```a2ui\s*\n/.test(text) && !/```openui\s*\n/.test(text);
}

/**
 * Parse an A2UI JSON block with fault tolerance.
 *
 * Attempts JSON.parse first. If that fails (e.g. the block is still streaming
 * and has a trailing unterminated string), falls back to a single best-effort
 * recovery pass (`tryPartialParse`) that truncates to the last syntactically
 * safe position and auto-closes unbalanced brackets. Returns `null` if the
 * recovery attempt also fails.
 */
export { parseA2UI } from "../../generative-ui/src/partial";
