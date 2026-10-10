"use client";

// Left rail of the Trace tab: go-to-turn, the session minimap built from
// server-side turn buckets, and the list of loaded turns.

import { useState } from "react";
import type { TraceItem, TraceOverview, TraceTurn } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { bucketIndex, bucketIntensity, formatClock, formatCount } from "./trace-model";

function sparkSegments(items: TraceItem[]): { key: string; grow: number; color: string }[] {
  return items.slice(0, 60).map((item, i) => {
    if (item.type === "gap") {
      return { key: `g${i}`, grow: 2, color: "bg-muted-foreground/20" };
    }
    if (item.type === "batch") {
      return {
        key: `b${i}`,
        grow: Math.min(40, Math.max(0.4, (item.wall_ms ?? 0) / 1000)),
        color: item.failed > 0 ? "bg-destructive" : "bg-accent",
      };
    }
    const color =
      item.status === "error"
        ? "bg-destructive"
        : item.kind === "model" || item.kind === "answer"
          ? "bg-primary/25"
          : item.kind === "agent" || item.kind === "send"
            ? "bg-info"
            : "bg-accent";
    return {
      key: `s${i}`,
      grow: Math.min(40, Math.max(0.4, (item.duration_ms ?? 0) / 1000)),
      color,
    };
  });
}

export function TurnRail({
  overview,
  turns,
  activeTurn,
  onJump,
  onScrollTo,
  goToRef,
}: {
  overview: TraceOverview | undefined;
  turns: TraceTurn[];
  activeTurn: number | null;
  onJump: (turn: number) => void;
  onScrollTo: (turn: number) => void;
  goToRef: React.RefObject<HTMLInputElement | null>;
}) {
  const [goTo, setGoTo] = useState("");
  const turnCount = overview?.turn_count ?? 0;
  const bucketSize = overview?.bucket_size ?? 1;
  const loadedBuckets = new Set(turns.map((t) => bucketIndex(t.turn, bucketSize)));
  const maxSteps = Math.max(0, ...(overview?.buckets.map((b) => b.steps) ?? [0]));

  return (
    <div className="flex flex-col gap-3 py-5 pl-6">
      <div className="flex items-baseline justify-between pr-1">
        <span className="text-[10px] font-medium tracking-[0.12em] text-muted-foreground">
          TURNS
        </span>
        <span className="font-mono text-[11px] text-muted-foreground">
          {formatCount(turnCount)}
        </span>
      </div>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          const turn = Number.parseInt(goTo, 10);
          if (Number.isFinite(turn) && turn >= 1 && turn <= turnCount) {
            onJump(turn);
            setGoTo("");
          }
        }}
        className="flex h-7 items-center border border-input bg-card px-2"
      >
        <input
          ref={goToRef}
          value={goTo}
          onChange={(e) => setGoTo(e.target.value.replace(/\D/g, ""))}
          inputMode="numeric"
          placeholder="Go to turn"
          aria-label="Go to turn"
          className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
        />
        <kbd className="font-mono text-[11px] text-muted-foreground">⌘G</kbd>
      </form>

      {overview && overview.buckets.length > 0 && (
        <div>
          <div className="grid grid-cols-14 gap-0.5" role="list" aria-label="Session minimap">
            {overview.buckets.map((bucket) => {
              const index = bucketIndex(bucket.from_turn, bucketSize);
              const loaded = loadedBuckets.has(index);
              const opacity = 0.12 + 0.45 * bucketIntensity(bucket.steps, maxSteps);
              return (
                <button
                  key={bucket.from_turn}
                  type="button"
                  role="listitem"
                  title={`Turns ${formatCount(bucket.from_turn)}–${formatCount(bucket.to_turn)}`}
                  aria-label={`Turns ${bucket.from_turn} to ${bucket.to_turn}${bucket.errors ? ", has errors" : ""}`}
                  onClick={() => onJump(bucket.from_turn)}
                  className="aspect-square"
                  style={{
                    background: loaded
                      ? "hsl(var(--primary))"
                      : bucket.errors > 0
                        ? "hsl(var(--destructive) / 0.75)"
                        : `hsl(var(--accent) / ${opacity.toFixed(2)})`,
                  }}
                />
              );
            })}
          </div>
          <div className="mt-1.5 flex justify-between pr-1 text-[11px] text-muted-foreground">
            <span>
              1 cell = {formatCount(bucketSize)} turn{bucketSize === 1 ? "" : "s"}
            </span>
            <span className="text-destructive">■ errors</span>
          </div>
        </div>
      )}

      {turns.length > 0 && (
        <div>
          <div className="mb-1 text-[10px] font-medium tracking-[0.12em] text-muted-foreground">
            LOADED
          </div>
          <ul>
            {turns.map((turn) => (
              <li key={turn.turn}>
                <button
                  type="button"
                  onClick={() => onScrollTo(turn.turn)}
                  className={cn(
                    "w-full border-l-2 border-transparent px-3 py-2 text-left hover:bg-muted",
                    activeTurn === turn.turn && "border-primary bg-primary/5",
                  )}
                >
                  <div className="flex items-center gap-2 font-mono text-[11px] text-muted-foreground">
                    <span>T{turn.turn}</span>
                    <span>{formatClock(turn.started_at)}</span>
                    {turn.error_count > 0 && (
                      <span className="size-1.5 bg-destructive" aria-label="has errors" />
                    )}
                  </div>
                  <div className="mt-0.5 line-clamp-2 text-[13px]">
                    {turn.prompt || "No prompt"}
                  </div>
                  <div className="mt-1.5 flex h-1 gap-px" aria-hidden>
                    {sparkSegments(turn.items).map((segment) => (
                      <span
                        key={segment.key}
                        className={segment.color}
                        style={{ flexGrow: segment.grow }}
                      />
                    ))}
                  </div>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
