"use client";

import { cn } from "@/lib/utils";

export type TimelineBin = {
  hour: string;
  ok?: number;
  err?: number;
  running?: number;
};

export function MiniTimeline({
  runs = [],
  length = 24,
  className,
}: {
  runs?: TimelineBin[];
  length?: number;
  className?: string;
}) {
  const bins = Array.from({ length }, (_, index) => runs[index] ?? { hour: String(index) });

  return (
    <div className={cn("flex items-end gap-1", className)} aria-label={`${length} hour timeline`}>
      {bins.map((bin, index) => {
        const ok = bin.ok ?? 0;
        const err = bin.err ?? 0;
        const running = bin.running ?? 0;
        const total = ok + err + running;
        return (
          <span
            key={`${bin.hour}-${index}`}
            title={`${bin.hour}: ${ok} ok, ${err} errors${running ? `, ${running} running` : ""}`}
            className={cn(
              "h-6 w-1.5 border",
              total === 0 && "border-muted bg-muted/40",
              ok > 0 && err === 0 && "border-emerald-200 bg-emerald-500",
              err > 0 && "border-red-200 bg-red-500",
              running > 0 && "border-amber-200 bg-amber-500",
            )}
          />
        );
      })}
    </div>
  );
}
