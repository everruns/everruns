"use client";

// Emoji reactions under a message: chips with counts, and a picker to add one
// (knowledge/execution/message-reactions.md, "Surfaces").
//
// Decisions:
// - Chips are the record. A person's reaction reaches the agent as a note,
//   which Chat does not show as a row; the trace shows it.
// - The caller's own chips are outlined; pressing one takes it back. The
//   agent's reactions show its name in the tooltip like anyone else's.
// - Good / Bad ratings stay separate: private, never sent to the agent.

import { SmilePlus } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useSessionReactions } from "@/hooks/use-message-reactions";
export { SessionReactionsScope } from "@/hooks/use-message-reactions";
import { QUICK_REACTIONS, type ReactionSummary } from "@/lib/api/message-reactions";
import { cn } from "@/lib/utils";
import { useLocale } from "@/providers/locale-provider";

/** Who reacted with one emoji, for the chip's tooltip. */
function reactorNames(reaction: ReactionSummary, you: string): string {
  const names = reaction.actors.map((actor) => actor.name || actor.kind);
  if (names.length === 0 && reaction.me) return you;
  return names.join(", ");
}

/** The picker: a quick row of emoji behind a small button. */
export function ReactionPicker({
  onPick,
  className,
  testId = "reaction-picker",
}: {
  onPick: (emoji: string) => void;
  className?: string;
  testId?: string;
}) {
  const { t } = useLocale();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={t("add_reaction")}
        title={t("add_reaction")}
        data-testid={testId}
        className={cn(
          "inline-flex size-6 shrink-0 cursor-pointer items-center justify-center text-muted-foreground outline-hidden hover:bg-muted/60 hover:text-foreground focus-visible:ring-1 focus-visible:ring-primary/30 data-[popup-open]:bg-muted/60",
          className,
        )}
      >
        <SmilePlus className="h-3 w-3" />
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="start" sideOffset={4}>
        <DropdownMenuContent className="flex min-w-0 flex-row gap-0.5 p-1">
          {QUICK_REACTIONS.map((emoji) => (
            <DropdownMenuItem
              key={emoji}
              aria-label={emoji}
              className="justify-center px-1.5 py-1 text-base"
              onClick={() => onPick(emoji)}
            >
              {emoji}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}

/**
 * Chips for one message. With `onToggle`, pressing a chip adds or removes the
 * caller's reaction and a picker follows the chips; without it they are read-only.
 */
export function ReactionChips({
  reactions,
  onToggle,
  className,
}: {
  reactions: ReactionSummary[];
  onToggle?: (emoji: string, add: boolean) => void;
  className?: string;
}) {
  const { t } = useLocale();
  if (reactions.length === 0 && !onToggle) return null;
  return (
    <div
      className={cn("flex flex-wrap items-center gap-1", className)}
      data-testid="message-reactions"
    >
      <TooltipProvider>
        {reactions.map((reaction) => {
          const label = t("reacted_with", {
            names: reactorNames(reaction, t("reaction_you")),
            emoji: reaction.emoji,
          });
          return (
            <Tooltip key={reaction.emoji}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  aria-pressed={reaction.me}
                  aria-label={
                    onToggle && reaction.me
                      ? t("reaction_remove", { emoji: reaction.emoji })
                      : label
                  }
                  disabled={!onToggle}
                  onClick={() => onToggle?.(reaction.emoji, !reaction.me)}
                  data-testid="reaction-chip"
                  className={cn(
                    "inline-flex h-6 items-center gap-1 border px-1.5 text-xs tabular-nums transition-colors",
                    reaction.me
                      ? "border-primary/70 bg-primary/5 text-foreground"
                      : "border-border/70 bg-background text-muted-foreground",
                    onToggle ? "cursor-pointer hover:bg-muted/60" : "cursor-default",
                  )}
                >
                  <span className="text-sm leading-none">{reaction.emoji}</span>
                  <span>{reaction.count}</span>
                </button>
              </TooltipTrigger>
              <TooltipContent>{label}</TooltipContent>
            </Tooltip>
          );
        })}
      </TooltipProvider>
      {onToggle && reactions.length > 0 && (
        <ReactionPicker
          onPick={(emoji) =>
            onToggle(emoji, !reactions.some((reaction) => reaction.emoji === emoji && reaction.me))
          }
          className="border border-border/70"
          testId="reaction-chip-add"
        />
      )}
    </div>
  );
}

/** Chips under an agent reply, from the session's reactions. */
export function MessageReactionChips({ messageId }: { messageId?: string }) {
  const session = useSessionReactions();
  if (!session || !messageId) return null;
  const reactions = session.reactionsOf(messageId);
  if (reactions.length === 0) return null;
  return (
    <ReactionChips
      reactions={reactions}
      onToggle={(emoji, add) => session.toggle(messageId, emoji, add)}
    />
  );
}

/** The reaction button in a reply's action row. Picking your own emoji takes it back. */
export function MessageReactionButton({ messageId }: { messageId?: string }) {
  const session = useSessionReactions();
  if (!session || !messageId) return null;
  return (
    <ReactionPicker
      onPick={(emoji) => {
        const own = session
          .reactionsOf(messageId)
          .some((reaction) => reaction.emoji === emoji && reaction.me);
        session.toggle(messageId, emoji, !own);
      }}
    />
  );
}

/**
 * Reactions under a message the person wrote: right-aligned chips, and a
 * picker that shows on hover until the message has a reaction.
 */
export function UserMessageReactions({ messageId }: { messageId?: string }) {
  const session = useSessionReactions();
  if (!session || !messageId) return null;
  const reactions = session.reactionsOf(messageId);
  const onToggle = (emoji: string, add: boolean) => session.toggle(messageId, emoji, add);
  return (
    <div className="-mt-1 flex justify-end">
      {reactions.length > 0 ? (
        <ReactionChips reactions={reactions} onToggle={onToggle} />
      ) : (
        <ReactionPicker
          onPick={(emoji) => onToggle(emoji, true)}
          className="opacity-0 group-hover/user-row:opacity-100 focus-visible:opacity-100 data-[popup-open]:opacity-100"
        />
      )}
    </div>
  );
}
