/**
 * OpenUI utilities for detecting and splitting OpenUI Lang code blocks
 * from markdown text in chat messages.
 *
 * OpenUI Lang code is wrapped in ```openui fenced code blocks by the LLM
 * when the openui capability is enabled. This module splits message text
 * into alternating markdown and openui segments for rendering.
 */

export interface MessageSegment {
  type: "markdown" | "openui";
  content: string;
}

/**
 * Regex matching ```openui ... ``` fenced code blocks.
 *
 * Handles:
 * - Opening: ```openui (with optional trailing whitespace/newline)
 * - Content: everything until closing backticks
 * - Closing: ``` on its own line (or end of string for streaming)
 */
const OPENUI_BLOCK_RE = /```openui\s*\n([\s\S]*?)(?:```|$)/g;

/**
 * Split message text into alternating markdown and openui segments.
 *
 * Given text like:
 *   "Here's a dashboard:\n```openui\nroot = Card([...])\n```\nLet me know!"
 *
 * Returns:
 *   [
 *     { type: "markdown", content: "Here's a dashboard:\n" },
 *     { type: "openui", content: "root = Card([...])\n" },
 *     { type: "markdown", content: "\nLet me know!" },
 *   ]
 */
export function splitOpenUIBlocks(text: string): MessageSegment[] {
  const segments: MessageSegment[] = [];
  let lastIndex = 0;

  // Reset regex state
  OPENUI_BLOCK_RE.lastIndex = 0;

  let match: RegExpExecArray | null;
  while ((match = OPENUI_BLOCK_RE.exec(text)) !== null) {
    // Add any markdown text before this block
    if (match.index > lastIndex) {
      const markdownContent = text.slice(lastIndex, match.index);
      if (markdownContent.trim()) {
        segments.push({ type: "markdown", content: markdownContent });
      }
    }

    // Add the openui block content (group 1 = content inside fences)
    const openUIContent = match[1];
    if (openUIContent.trim()) {
      segments.push({ type: "openui", content: openUIContent });
    }

    lastIndex = match.index + match[0].length;
  }

  // Add any trailing markdown text
  if (lastIndex < text.length) {
    const trailing = text.slice(lastIndex);
    if (trailing.trim()) {
      segments.push({ type: "markdown", content: trailing });
    }
  }

  // If no openui blocks found, return the whole thing as markdown
  if (segments.length === 0 && text.trim()) {
    segments.push({ type: "markdown", content: text });
  }

  return segments;
}

/**
 * Check if text contains any openui code blocks.
 */
export function hasOpenUIBlocks(text: string): boolean {
  return /```openui\s*\n/.test(text);
}
