import { cn } from "@/lib/utils";

export type DotTone = "live" | "draft" | "paused" | "muted";

/** A small state marker: filled green for live, hollow for draft, red for paused. */
export function StateDot({ tone, label }: { tone: DotTone; label?: string }) {
  return (
    <span
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={cn(
        "inline-block size-1.5 shrink-0 rounded-full",
        tone === "live" && "bg-success",
        tone === "paused" && "bg-destructive",
        tone === "muted" && "bg-muted-foreground/50",
        tone === "draft" && "border border-muted-foreground/70",
      )}
    />
  );
}
