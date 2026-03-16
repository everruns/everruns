/**
 * Decision: Keep the canonical chat surface chrome in one place so runtime chat
 * and dev previews stay visually aligned instead of hand-copying class strings.
 */

export const chatSurfaceStyles = {
  emptyStateCard:
    "animate-chat-surface-in mx-auto mb-8 w-full max-w-md border border-border/70 bg-card/90 px-8 py-8 text-center shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]",
  userMessage:
    "animate-chat-row-in max-w-[min(78%,42rem)] border-r border-r-accent/75 bg-[hsl(var(--accent)/0.09)] px-4 py-3.5 text-sm text-foreground shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]",
  agentMessageRow: "flex w-full items-start gap-3 pr-2",
  agentIcon:
    "mt-1 flex h-6 w-6 flex-shrink-0 items-center justify-center border border-border/70 bg-primary text-primary-foreground shadow-[inset_0_1px_0_hsl(var(--background)/0.12)]",
  agentMessage:
    "animate-chat-row-in flex-1 space-y-2 border-l border-l-primary/65 bg-card/90 px-4 py-3.5 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]",
  composerSection: "bg-background/55 px-4 pb-4 pt-3 backdrop-blur-[1px] sm:px-5",
  composerInputShell:
    "relative overflow-hidden border border-border/70 bg-background/95 shadow-[0_1px_0_hsl(var(--background)/0.88)] transition-[background-color,border-color,box-shadow] duration-200 focus-within:border-primary/30 focus-within:ring-1 focus-within:ring-primary/10",
  composerControlChip:
    "flex h-9 items-center gap-2 border border-border/70 bg-card/85 px-2.5 text-sm shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]",
  composerIconButton: "h-9 w-9 border-border/70 bg-card/85 shadow-none hover:bg-muted/40",
  composerDangerButton:
    "h-9 w-9 border border-destructive/30 bg-destructive/[0.08] text-destructive shadow-none hover:bg-destructive/[0.14]",
  composerSubmitButton: "h-9 w-9 shadow-[inset_0_1px_0_hsl(var(--primary-foreground)/0.1)]",
  composerTextarea:
    "min-h-[96px] max-h-[220px] w-full resize-none border-0 bg-transparent px-4 py-3.5 text-sm shadow-none focus-visible:ring-0",
  floatingNotice:
    "sticky bottom-3 left-1/2 z-10 mx-auto flex -translate-x-1/2 items-center gap-1.5 border border-border/70 bg-card/90 px-3 py-1.5 text-xs font-medium text-foreground shadow-md transition-colors hover:bg-accent/10",
} as const;
