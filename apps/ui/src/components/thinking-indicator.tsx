"use client";

/**
 * ThinkingIndicator - Animated indicator shown while agent is generating
 *
 * Shows a bouncing animation with 3 small dots to indicate the agent is "thinking"
 * (generating a response). Used before any streaming text arrives.
 */

import { cn } from "@/lib/utils";
import { useLocale } from "@/providers/locale-provider";

interface ThinkingIndicatorProps {
  className?: string;
  /** Optional model name being used */
  model?: string;
}

export function ThinkingIndicator({ className, model }: ThinkingIndicatorProps) {
  const { t } = useLocale();
  return (
    <div className={cn("flex items-center gap-1", className)}>
      <span className="text-xs text-muted-foreground/60">{t("thinking")}</span>
      {model && <span className="text-xs text-muted-foreground/40">{model}</span>}
      <span className="flex gap-0.5 ml-0.5">
        <span
          className="w-1 h-1 bg-muted-foreground/40 rounded-full animate-bounce"
          style={{ animationDelay: "0ms" }}
        />
        <span
          className="w-1 h-1 bg-muted-foreground/40 rounded-full animate-bounce"
          style={{ animationDelay: "150ms" }}
        />
        <span
          className="w-1 h-1 bg-muted-foreground/40 rounded-full animate-bounce"
          style={{ animationDelay: "300ms" }}
        />
      </span>
    </div>
  );
}
