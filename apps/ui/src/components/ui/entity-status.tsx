"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";
import { cn } from "@/lib/utils";

export type EntityStatusTone = "success" | "warning" | "destructive" | "muted";

const DOT: Record<EntityStatusTone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  destructive: "bg-destructive",
  muted: "bg-muted-foreground/60",
};

function lifecycleTone(status: string): EntityStatusTone {
  const variant = getEntityStatusBadgeVariant(status);
  return variant === "success" ? "success" : variant === "destructive" ? "destructive" : "muted";
}

/**
 * Lifecycle is readable as text; color is a secondary cue on overview surfaces.
 * `tone` and `label` cover states outside the entity lifecycle (sync, indexing,
 * extraction); `label` is shown in sentence case as given.
 */
export function EntityStatus({
  status,
  tone,
  label,
  className,
}: {
  status: string;
  tone?: EntityStatusTone;
  label?: string;
  className?: string;
}) {
  const ui = useUiTranslations();
  const fallback = status.replaceAll("_", " ");
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground",
        className,
      )}
    >
      <span
        aria-hidden="true"
        className={cn("size-1.5 rounded-full", DOT[tone ?? lifecycleTone(status)])}
      />
      {label !== undefined ? (
        <span>{ui(label)}</span>
      ) : (
        <span className="capitalize">
          {ui(fallback.charAt(0).toUpperCase() + fallback.slice(1)).toLowerCase()}
        </span>
      )}
    </span>
  );
}
