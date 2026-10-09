import type { RunBucket } from "@/lib/api/types";
import { cn } from "@/lib/utils";

/**
 * A strip of small bars, one per bucket, oldest on the left. A bucket with a
 * failure is drawn in the destructive color so a bad hour stands out without
 * a legend; an empty bucket keeps a hairline so the strip's length still reads.
 */
export function RunBars({
  buckets,
  label,
  className,
}: {
  buckets: Array<Pick<RunBucket, "runs" | "failed">>;
  /** Accessible summary of what the bars show. */
  label: string;
  className?: string;
}) {
  const max = Math.max(1, ...buckets.map((bucket) => bucket.runs));
  return (
    <div role="img" aria-label={label} className={cn("flex h-6 items-end gap-px", className)}>
      {buckets.map((bucket, index) => {
        const height = bucket.runs === 0 ? 0 : Math.max(18, Math.round((bucket.runs / max) * 100));
        return (
          <span
            // Buckets are positional; the index is their identity.
            key={index}
            className={cn(
              "w-1 flex-1",
              bucket.runs === 0
                ? "h-px bg-border"
                : bucket.failed > 0
                  ? "bg-destructive"
                  : "bg-muted-foreground/55",
            )}
            style={bucket.runs === 0 ? undefined : { height: `${height}%` }}
          />
        );
      })}
    </div>
  );
}
