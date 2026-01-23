"use client";

/**
 * StreamingThinking - Displays streaming thinking/reasoning content from extended thinking models
 *
 * Shows the accumulated thinking from thinking.delta events in a collapsible section.
 * This is the model's chain-of-thought reasoning before producing the final response.
 */

import { useState } from "react";
import { ChevronDown, ChevronRight, Brain } from "lucide-react";
import { cn } from "@/lib/utils";

interface StreamingThinkingProps {
  text: string;
  className?: string;
  /** Whether to start in collapsed state */
  defaultCollapsed?: boolean;
  /** Whether the model is still generating thinking content */
  isStreaming?: boolean;
}

export function StreamingThinking({
  text,
  className,
  defaultCollapsed = false,
  isStreaming = false,
}: StreamingThinkingProps) {
  const [isCollapsed, setIsCollapsed] = useState(defaultCollapsed);

  return (
    <div
      className={cn(
        "border border-muted-foreground/20 rounded-lg overflow-hidden",
        className
      )}
    >
      {/* Header - clickable to toggle collapse */}
      <button
        type="button"
        onClick={() => setIsCollapsed(!isCollapsed)}
        className="w-full flex items-center gap-2 px-3 py-2 bg-muted/30 hover:bg-muted/50 transition-colors text-left"
      >
        {isCollapsed ? (
          <ChevronRight className="w-4 h-4 text-muted-foreground flex-shrink-0" />
        ) : (
          <ChevronDown className="w-4 h-4 text-muted-foreground flex-shrink-0" />
        )}
        <Brain className="w-4 h-4 text-muted-foreground flex-shrink-0" />
        <span className="text-sm font-medium text-muted-foreground">
          Thinking
        </span>
        {isStreaming && (
          <span className="relative flex h-2 w-2 ml-1">
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-muted-foreground/50 opacity-75" />
            <span className="relative inline-flex rounded-full h-2 w-2 bg-muted-foreground/50" />
          </span>
        )}
      </button>

      {/* Collapsible content */}
      {!isCollapsed && (
        <div className="px-3 py-2 bg-muted/10">
          <div className="text-sm text-muted-foreground whitespace-pre-wrap font-mono">
            {text}
            {/* Blinking cursor when streaming */}
            {isStreaming && (
              <span className="inline-block w-0.5 h-4 ml-0.5 bg-muted-foreground/50 animate-pulse align-text-bottom" />
            )}
          </div>
        </div>
      )}
    </div>
  );
}
