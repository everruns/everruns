"use client";

/**
 * ThinkingIndicator - Animated indicator shown while agent is generating
 *
 * Shows a bouncing animation with 3 dots to indicate the agent is "thinking"
 * (generating a response). Used before any streaming text arrives.
 */

import { cn } from "@/lib/utils";

interface ThinkingIndicatorProps {
  className?: string;
  /** Optional model name being used */
  model?: string;
}

export function ThinkingIndicator({ className, model }: ThinkingIndicatorProps) {
  return (
    <div className={cn("flex items-center gap-2 text-muted-foreground py-2", className)}>
      <div className="flex items-center gap-1">
        {/* Three bouncing dots with staggered animation using CSS keyframes */}
        <style>
          {`
            @keyframes thinking-bounce {
              0%, 60%, 100% { transform: translateY(0); opacity: 0.4; }
              30% { transform: translateY(-4px); opacity: 1; }
            }
            .thinking-dot {
              animation: thinking-bounce 1.2s ease-in-out infinite;
            }
            .thinking-dot:nth-child(2) { animation-delay: 0.15s; }
            .thinking-dot:nth-child(3) { animation-delay: 0.3s; }
          `}
        </style>
        <span className="thinking-dot inline-block h-2 w-2 rounded-full bg-primary" />
        <span className="thinking-dot inline-block h-2 w-2 rounded-full bg-primary" />
        <span className="thinking-dot inline-block h-2 w-2 rounded-full bg-primary" />
      </div>
      <span className="text-sm">
        {model ? `Thinking with ${model}...` : "Thinking..."}
      </span>
    </div>
  );
}
