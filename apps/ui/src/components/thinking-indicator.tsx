"use client";

/**
 * ThinkingIndicator — Combo wave bar animation (fast + navy-to-gold gradient)
 *
 * Shows an animated equalizer-style wave bar while the agent is generating.
 * Uses brand colors: navy → gold in light mode, slate → gold in dark mode.
 * Relies on the `loading-wave-bar` keyframes in globals.css.
 */

import { cn } from "@/lib/utils";
import { useLocale } from "@/providers/locale-provider";

const BAR_DELAYS = [0, 80, 160, 240, 320];

interface ThinkingIndicatorProps {
  className?: string;
  /** Optional model name being used */
  model?: string;
}

export function ThinkingIndicator({ className, model }: ThinkingIndicatorProps) {
  const { t } = useLocale();
  return (
    <div className={cn("flex items-center gap-2", className)}>
      <div className="flex items-end gap-[2px] h-4">
        {BAR_DELAYS.map((delay, i) => (
          <span
            key={delay}
            className={`loading-wave-bar w-[3px] rounded-[1px] loading-wave-bar-${i}`}
            style={{
              animationDelay: `${delay}ms`,
              animationDuration: "0.7s",
            }}
          />
        ))}
      </div>
      <span className="text-xs text-accent-foreground/60">
        {t("thinking")}
        {model && <span className="ml-1 text-muted-foreground/40">{model}</span>}
      </span>
    </div>
  );
}
