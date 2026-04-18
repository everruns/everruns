/**
 * A2UI utilities for detecting and splitting A2UI JSON code blocks from
 * markdown text in chat messages.
 *
 * A2UI JSON is wrapped in ```a2ui fenced code blocks by the LLM when the
 * `a2ui` capability is enabled. This module splits message text into
 * alternating markdown, openui, and a2ui segments so the renderer can
 * dispatch each to its own component.
 *
 * @see specs/a2ui.md
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
  return /```(?:a2ui|openui)\s*\n/.test(text);
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
export function parseA2UI(code: string): unknown {
  const trimmed = code.trim();
  if (!trimmed) return null;

  try {
    return JSON.parse(trimmed);
  } catch {
    // Attempt progressive truncation for streaming tolerance.
    return tryPartialParse(trimmed);
  }
}

/**
 * Try to recover a parseable JSON prefix by closing unbalanced brackets.
 * This is best-effort: if it cannot produce valid JSON, returns null.
 */
function tryPartialParse(text: string): unknown {
  // Walk the text tracking brackets + string state; emit a "closed" version
  // by padding the right number of close brackets.
  let inString = false;
  let escape = false;
  const stack: Array<"{" | "["> = [];
  let lastSafeIndex = -1;

  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (escape) {
      escape = false;
      continue;
    }
    if (ch === "\\") {
      if (inString) escape = true;
      continue;
    }
    if (ch === '"') {
      inString = !inString;
      if (!inString) lastSafeIndex = i;
      continue;
    }
    if (inString) continue;

    if (ch === "{" || ch === "[") {
      stack.push(ch);
    } else if (ch === "}" || ch === "]") {
      stack.pop();
      lastSafeIndex = i;
    } else if (!/\s/.test(ch) && stack.length === 0) {
      // After the top-level object closes, anything else is garbage.
      break;
    } else if (stack.length > 0) {
      lastSafeIndex = i;
    }
  }

  if (lastSafeIndex < 0) return null;

  // Truncate to last safe point and close remaining brackets.
  let truncated = text.slice(0, lastSafeIndex + 1);

  // If we stopped mid-key/value (e.g. "foo": ), strip trailing comma/colon.
  truncated = truncated.replace(/[,:\s]+$/, "");

  // Re-scan truncated to recount open brackets.
  const closes: string[] = [];
  let inStr = false;
  let esc = false;
  for (const ch of truncated) {
    if (esc) {
      esc = false;
      continue;
    }
    if (ch === "\\") {
      if (inStr) esc = true;
      continue;
    }
    if (ch === '"') {
      inStr = !inStr;
      continue;
    }
    if (inStr) continue;
    if (ch === "{") closes.push("}");
    else if (ch === "[") closes.push("]");
    else if (ch === "}" || ch === "]") closes.pop();
  }

  const candidate = truncated + closes.reverse().join("");
  try {
    return JSON.parse(candidate);
  } catch {
    return null;
  }
}
