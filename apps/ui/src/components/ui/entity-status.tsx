import { getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";
import { cn } from "@/lib/utils";

/** Lifecycle is readable as text; color is a secondary cue on overview surfaces. */
export function EntityStatus({ status }: { status: string }) {
  const variant = getEntityStatusBadgeVariant(status);
  return (
    <span className="inline-flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground">
      <span
        aria-hidden="true"
        className={cn(
          "size-1.5 rounded-full",
          variant === "success"
            ? "bg-success"
            : variant === "destructive"
              ? "bg-destructive"
              : "bg-muted-foreground/60",
        )}
      />
      <span className="capitalize">{status.replaceAll("_", " ")}</span>
    </span>
  );
}
