"use client";

/**
 * ThinkingContent - Displays LLM thinking/reasoning content
 *
 * Shows the model's reasoning process in a collapsible section.
 * Used for:
 * 1. Streaming thinking content during generation (thinking.delta events)
 * 2. Final thinking content in messages (thinking content parts)
 *
 * The content is collapsed by default to not overwhelm the user,
 * but can be expanded to see the full reasoning process.
 */

import { useState } from "react";
import { cn } from "@/lib/utils";
import { ChevronDown, ChevronRight, Brain } from "lucide-react";

interface ThinkingContentProps {
  /** The thinking/reasoning text content */
  thinking: string;
  /** Whether this is streaming (shows animated indicator) */
  isStreaming?: boolean;
  /** Additional CSS classes */
  className?: string;
  /** Whether to start expanded (default: false) */
  defaultExpanded?: boolean;
}

export function ThinkingContent({
  thinking,
  isStreaming = false,
  className,
  defaultExpanded = false,
}: ThinkingContentProps) {
  const [isExpanded, setIsExpanded] = useState(defaultExpanded);

  // Count lines/characters for preview
  const lineCount = thinking.split("\n").length;
  const charCount = thinking.length;

  return (
    <div
      className={cn(
        "border border-amber-200/50 dark:border-amber-800/50 rounded-lg bg-amber-50/50 dark:bg-amber-950/20",
        className
      )}
    >
      {/* Header - always visible */}
      <button
        type="button"
        onClick={() => setIsExpanded(!isExpanded)}
        className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-amber-100/50 dark:hover:bg-amber-900/30 transition-colors rounded-t-lg"
      >
        {/* Expand/collapse icon */}
        {isExpanded ? (
          <ChevronDown className="h-4 w-4 text-amber-600 dark:text-amber-400 flex-shrink-0" />
        ) : (
          <ChevronRight className="h-4 w-4 text-amber-600 dark:text-amber-400 flex-shrink-0" />
        )}

        {/* Brain icon */}
        <Brain className="h-4 w-4 text-amber-600 dark:text-amber-400 flex-shrink-0" />

        {/* Label */}
        <span className="text-sm font-medium text-amber-700 dark:text-amber-300">
          Thinking
        </span>

        {/* Streaming indicator */}
        {isStreaming && (
          <span className="flex items-center gap-1 text-xs text-amber-600 dark:text-amber-400">
            <span className="relative flex h-2 w-2">
              <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-500 opacity-75" />
              <span className="relative inline-flex rounded-full h-2 w-2 bg-amber-500" />
            </span>
          </span>
        )}

        {/* Stats (when collapsed) */}
        {!isExpanded && (
          <span className="text-xs text-amber-600/60 dark:text-amber-400/60 ml-auto">
            {lineCount > 1 ? `${lineCount} lines` : `${charCount} chars`}
          </span>
        )}
      </button>

      {/* Content - shown when expanded */}
      {isExpanded && (
        <div className="px-3 pb-3">
          <div className="text-sm text-amber-800/80 dark:text-amber-200/80 whitespace-pre-wrap font-mono text-xs leading-relaxed">
            {thinking}
            {/* Streaming cursor */}
            {isStreaming && (
              <span className="inline-block w-0.5 h-3 ml-0.5 bg-amber-600/70 dark:bg-amber-400/70 animate-pulse align-text-bottom" />
            )}
          </div>
        </div>
      )}
    </div>
  );
}
