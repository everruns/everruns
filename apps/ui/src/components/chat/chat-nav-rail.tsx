/**
 * Turn navigation rail — a gutter of turn markers pinned to the right edge of
 * the transcript. Hover a marker to preview that turn's prompt; click to jump.
 *
 * Adopts the navigation-rail idea from shadcn/ui's MessageScroller (June 2026
 * chat components), reimplemented on the Slate design system: sharp corners,
 * gold accent for the active turn, border tones for the rest.
 *
 * The active marker is driven by `useMessageScrollerVisibility`; jumping is a
 * plain `scrollIntoView` on the anchored row.
 */
"use client";

import { cn } from "@/lib/utils";
import { useLocale } from "@/providers/locale-provider";

export interface ChatNavAnchor {
  id: string;
  label: string;
}

interface ChatNavRailProps {
  anchors: ChatNavAnchor[];
  currentAnchorId: string | null;
  onJump: (id: string) => void;
}

export function ChatNavRail({ anchors, currentAnchorId, onJump }: ChatNavRailProps) {
  const { t } = useLocale();

  // A single turn needs no navigation.
  if (anchors.length < 2) return null;

  return (
    <nav
      aria-label={t("conversation_turns")}
      // Offset from the right edge so markers clear the transcript scrollbar.
      className="pointer-events-none absolute inset-y-0 right-0 z-10 hidden flex-col items-end justify-center overflow-y-auto py-6 pr-3 md:flex"
    >
      {anchors.map((anchor) => {
        const isCurrent = anchor.id === currentAnchorId;
        const label = anchor.label.trim() || t("jump_to_turn");
        return (
          <button
            key={anchor.id}
            type="button"
            onClick={() => onJump(anchor.id)}
            aria-label={`${t("jump_to_turn")}: ${label}`}
            aria-current={isCurrent ? "true" : undefined}
            title={label}
            // Generous vertical padding gives each thin marker a comfortable
            // hover/click target without thickening the visible tick.
            className="group pointer-events-auto flex items-center justify-end gap-2 py-1"
          >
            <span
              className={cn(
                "max-w-0 overflow-hidden whitespace-nowrap text-[11px] leading-4 text-foreground opacity-0 transition-all duration-150",
                "group-hover:max-w-[16rem] group-hover:border group-hover:border-border/70 group-hover:bg-card/95 group-hover:px-2 group-hover:py-0.5 group-hover:opacity-100 group-focus-visible:max-w-[16rem] group-focus-visible:border group-focus-visible:border-border/70 group-focus-visible:bg-card/95 group-focus-visible:px-2 group-focus-visible:py-0.5 group-focus-visible:opacity-100",
              )}
            >
              {label}
            </span>
            <span
              className={cn(
                "h-0.5 flex-shrink-0 transition-all duration-150",
                isCurrent
                  ? "w-5 bg-accent"
                  : "w-3 bg-border group-hover:w-5 group-hover:bg-muted-foreground",
              )}
            />
          </button>
        );
      })}
    </nav>
  );
}
