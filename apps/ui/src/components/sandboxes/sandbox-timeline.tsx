"use client";

// One lane per Sandbox over a window: solid while running, hatched while
// paused, red while lost, with a tick where a lost resource was rebuilt.
// Underneath, a step chart of how many ran at once.

import type { SandboxTimeline as Timeline, SandboxTimelineLane } from "@/lib/api/sandboxes";
import { cn } from "@/lib/utils";
import { formatRunningTime, providerLabel, sandboxStateLabel } from "./sandbox-display";
import { sandboxTitle } from "./sandbox-fleet-table";

const TICKS = 8;

function position(at: string, from: number, span: number): number {
  return Math.min(100, Math.max(0, ((new Date(at).getTime() - from) / span) * 100));
}

function tickLabel(time: number, span: number): string {
  const date = new Date(time);
  return span > 2 * 86_400_000
    ? date.toLocaleDateString(undefined, { weekday: "short", day: "numeric" })
    : date.toLocaleTimeString(undefined, {
        hour: "2-digit",
        minute: "2-digit",
      });
}

const SEGMENT_CLASS: Record<string, string> = {
  running: "bg-success",
  // Hatching keeps paused readable without relying on color.
  paused:
    "border border-warning bg-[repeating-linear-gradient(45deg,hsl(var(--warning))_0_3px,transparent_3px_6px)]",
  lost: "bg-destructive",
  failed: "bg-destructive",
  starting: "bg-accent",
};

function Lane({
  lane,
  from,
  span,
  onSelect,
}: {
  lane: SandboxTimelineLane;
  from: number;
  span: number;
  onSelect: (id: string) => void;
}) {
  const { sandbox } = lane;
  return (
    <div className="grid grid-cols-[minmax(0,13rem)_1fr] items-center gap-3">
      <button
        type="button"
        onClick={() => onSelect(sandbox.id)}
        className="min-w-0 truncate text-left text-sm hover:underline"
        title={`${sandboxTitle(sandbox)}, ${formatRunningTime(lane.running_seconds)} running`}
      >
        {sandboxTitle(sandbox)}
        <span className="text-muted-foreground"> · {providerLabel(sandbox.provider)}</span>
      </button>
      <div className="relative h-4" role="img" aria-label={`${sandboxTitle(sandbox)} lifecycle`}>
        {lane.spans.map((spanItem, index) => {
          const left = position(spanItem.start, from, span);
          const width = Math.max(0.3, position(spanItem.end, from, span) - left);
          const previous = lane.spans[index - 1];
          const rebuilt = previous && spanItem.generation > previous.generation;
          return (
            <span key={`${spanItem.start}-${index}`}>
              {rebuilt ? (
                <span
                  className="absolute inset-y-0 w-0.5 bg-destructive"
                  style={{ left: `${left}%` }}
                  title={`Rebuilt as generation ${spanItem.generation}`}
                />
              ) : null}
              <span
                className={cn(
                  "absolute top-0.5 h-3",
                  SEGMENT_CLASS[spanItem.state] ?? "bg-muted-foreground/40",
                )}
                style={{ left: `${left}%`, width: `${width}%` }}
                title={`${sandboxStateLabel(spanItem.state)} ${new Date(spanItem.start).toLocaleString()}`}
              />
            </span>
          );
        })}
      </div>
    </div>
  );
}

function Concurrency({ timeline, from, span }: { timeline: Timeline; from: number; span: number }) {
  const peak = Math.max(1, timeline.peak_running);
  const height = 48;
  const points = timeline.concurrency;
  let path = "";
  points.forEach((point, index) => {
    const x = position(point.at, from, span) * 10;
    const y = height - (point.running / peak) * (height - 4);
    if (index === 0) path += `M${x} ${y}`;
    else {
      const prevY = height - (points[index - 1].running / peak) * (height - 4);
      path += ` L${x} ${prevY} L${x} ${y}`;
    }
  });
  const last = points[points.length - 1];
  if (last) path += ` L1000 ${height - (last.running / peak) * (height - 4)}`;
  const peakX = timeline.peak_at ? position(timeline.peak_at, from, span) * 10 : null;
  return (
    <div className="grid grid-cols-[minmax(0,13rem)_1fr] items-end gap-3 border-t pt-3">
      <span className="text-xs text-muted-foreground">
        Running at once
        <span className="block">
          {timeline.peak_at
            ? `Peak ${timeline.peak_running} at ${new Date(timeline.peak_at).toLocaleString()}`
            : "None in this window"}
        </span>
      </span>
      <svg
        viewBox={`0 0 1000 ${height}`}
        preserveAspectRatio="none"
        className="h-12 w-full text-success"
        role="img"
        aria-label={`Concurrent running Sandboxes, peak ${timeline.peak_running}`}
      >
        <line x1="0" y1={height - 0.5} x2="1000" y2={height - 0.5} className="stroke-border" />
        {path ? (
          <>
            <path
              d={`${path} L1000 ${height} L0 ${height} Z`}
              fill="currentColor"
              fillOpacity={0.12}
            />
            <path
              d={path}
              fill="none"
              stroke="currentColor"
              strokeWidth={1.5}
              vectorEffect="non-scaling-stroke"
            />
          </>
        ) : null}
        {peakX !== null ? <circle cx={peakX} cy={4} r={3} fill="currentColor" /> : null}
      </svg>
    </div>
  );
}

export function SandboxTimelineChart({
  timeline,
  onSelect,
}: {
  timeline: Timeline;
  onSelect: (id: string) => void;
}) {
  const from = new Date(timeline.from).getTime();
  const span = Math.max(1, new Date(timeline.to).getTime() - from);
  const ticks = Array.from({ length: TICKS + 1 }, (_, i) => from + (span / TICKS) * i);
  return (
    <div className="flex flex-col gap-2 overflow-x-auto">
      <div className="flex min-w-[44rem] flex-col gap-2">
        <div className="grid grid-cols-[minmax(0,13rem)_1fr] gap-3">
          <span />
          <div className="flex justify-between font-mono text-[11px] text-muted-foreground">
            {ticks.map((tick, i) => (
              <span key={tick}>{i === TICKS ? "now" : tickLabel(tick, span)}</span>
            ))}
          </div>
        </div>
        {timeline.lanes.map((lane) => (
          <Lane key={lane.sandbox.id} lane={lane} from={from} span={span} onSelect={onSelect} />
        ))}
        <Concurrency timeline={timeline} from={from} span={span} />
        <div className="flex flex-wrap gap-4 pt-1 text-xs text-muted-foreground">
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2 w-3 bg-success" /> Running
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className={cn("h-2 w-3", SEGMENT_CLASS.paused)} /> Paused
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2 w-3 bg-destructive" /> Lost
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className="h-3 w-0.5 bg-destructive" /> Rebuilt
          </span>
          {timeline.total_lanes > timeline.lanes.length ? (
            <span>
              Showing the {timeline.lanes.length} longest-running of {timeline.total_lanes}
            </span>
          ) : null}
        </div>
      </div>
    </div>
  );
}
