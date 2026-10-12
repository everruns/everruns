/**
 * Decision: Keep the canonical chat surface chrome in one place so runtime chat
 * and dev previews stay visually aligned instead of hand-copying class strings.
 *
 * Decision (knowledge/ui/chat-experience.md): transcript and composer share one
 * centered 820px column. Agent replies are plain prose with no card or avatar;
 * user turns are a gold-wash block with a right accent rule.
 */

export const chatSurfaceStyles = {
  emptyStateCard:
    "animate-chat-surface-in mx-auto mb-6 w-full max-w-md border border-border/70 bg-card/90 px-6 py-6 text-center shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]",
  column: "mx-auto w-full max-w-[820px]",
  userMessage:
    "animate-chat-row-in max-w-[min(78%,42rem)] border-r-2 border-r-accent bg-[hsl(var(--accent)/0.12)] px-3.5 py-2.5 text-sm leading-[1.6] text-foreground",
  agentMessageRow: "group/agent-reply flex w-full min-w-0 flex-col gap-1",
  agentMessage: "animate-chat-row-in min-w-0 space-y-1.5 text-sm leading-[1.6] text-foreground",
  /** Copy and details under a reply: always shown on the latest, on hover for earlier ones. */
  agentActions:
    "-ml-1 flex h-7 items-center gap-0.5 text-muted-foreground transition-opacity duration-150 focus-within:opacity-100 group-hover/agent-reply:opacity-100",
  daySeparator: "py-2 text-center text-xs font-medium text-muted-foreground",
  composerSection:
    "mx-auto w-full max-w-[820px] bg-background/55 px-3 pb-3 pt-2.5 backdrop-blur-[1px] sm:px-6",
  composerInputShell:
    "relative border border-border/70 bg-background/95 shadow-[0_1px_0_hsl(var(--background)/0.88)] transition-[background-color,border-color,box-shadow] duration-200 focus-within:border-primary/30 focus-within:ring-1 focus-within:ring-primary/10",
  composerControlChip:
    "flex h-8 items-center gap-1.5 border border-border/70 bg-card/85 px-2 text-[13px] shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]",
  composerIconButton: "h-8 w-8 border-border/70 bg-card/85 shadow-none hover:bg-muted/40",
  composerDangerButton:
    "h-8 w-8 border border-destructive/30 bg-destructive/[0.08] text-destructive shadow-none hover:bg-destructive/[0.14]",
  composerSubmitButton: "h-8 w-8 shadow-[inset_0_1px_0_hsl(var(--primary-foreground)/0.1)]",
  composerTextarea:
    "min-h-[84px] max-h-[200px] w-full resize-none border-0 bg-transparent px-3 py-2.5 text-sm leading-[1.6] shadow-none focus-visible:ring-0",
  floatingNotice:
    "sticky bottom-3 left-1/2 z-10 mx-auto flex -translate-x-1/2 items-center gap-1.5 border border-border/70 bg-card/90 px-2.5 py-1 text-[11px] font-medium text-foreground shadow-md transition-colors hover:bg-accent/10",
} as const;
