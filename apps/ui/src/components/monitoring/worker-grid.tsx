"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useMemo } from "react";
import type { MonitoringSnapshot, MonitoringWorker } from "@/lib/api/types";
import { phases, number } from "./model";
import { cn } from "@/lib/utils";

export interface WorkerGroup {
  id: string;
  name: string;
  workers: MonitoringWorker[];
  leases: number;
  phases: Record<string, number>;
  load: number | null;
  capacity: number | null;
  stale: number;
}
export function groupWorkers(workers: MonitoringWorker[], size: number): WorkerGroup[] {
  const pools = new Map<string, MonitoringWorker[]>();
  for (const worker of [...workers].sort((a, b) => a.id.localeCompare(b.id))) {
    const name = worker.worker_group || "Workers";
    const members = pools.get(name) ?? [];
    members.push(worker);
    pools.set(name, members);
  }
  return [...pools]
    .sort(([a], [b]) => a.localeCompare(b))
    .flatMap(([name, pool]) => {
      const groups: WorkerGroup[] = [];
      for (let start = 0; start < pool.length; start += size) {
        const members = pool.slice(start, start + size);
        const known = members.filter(
          (w) => w.capacity != null && w.status !== "stale" && w.status !== "stopped",
        );
        const counts: Record<string, number> = {};
        for (const w of members)
          for (const [phase, count] of Object.entries(w.phases))
            counts[phase] = (counts[phase] ?? 0) + count;
        groups.push({
          id: `${name}:${start}`,
          name: size === 1 ? members[0].id : `${name} · ${Math.floor(start / size) + 1}`,
          workers: members,
          leases: members.reduce((n, w) => n + w.actor_leases, 0),
          phases: counts,
          load: known.length ? known.reduce((n, w) => n + (w.current_load ?? 0), 0) : null,
          capacity: known.length ? known.reduce((n, w) => n + (w.capacity ?? 0), 0) : null,
          stale: members.filter((w) => w.status === "stale").length,
        });
      }
      return groups;
    });
}

export function WorkerGrid({
  data,
  size,
  selected,
  onSelect,
}: {
  data: MonitoringSnapshot;
  size: number;
  selected: string | null;
  onSelect: (group: WorkerGroup) => void;
}) {
  const ui = useUiTranslations();
  const groups = useMemo(() => groupWorkers(data.workers, size), [data.workers, size]);
  return (
    <section className="border bg-card p-4" data-testid="monitoring-workers">
      <div className="mb-4 flex flex-wrap items-baseline justify-between gap-2">
        <h2 className="text-sm font-semibold">{ui("Work distributed over workers")}</h2>
        <span className="text-xs text-muted-foreground">
          {number(data.workers.length)} {ui("workers ·")} {groups.length} {ui("groups")}
        </span>
      </div>
      {groups.length === 0 ? (
        <p className="py-12 text-center text-sm text-muted-foreground">
          {ui("No worker ownership has been observed in this scope.")}
        </p>
      ) : (
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4 2xl:grid-cols-6">
          {groups.map((group) => {
            let cumulative = 0;
            const ranges = phases.map((p) => {
              const start = cumulative;
              cumulative += group.phases[p.key] ?? 0;
              return { ...p, start, end: cumulative };
            });
            const load = group.capacity
              ? Math.min(100, ((group.load ?? 0) / group.capacity) * 100)
              : 0;
            return (
              <button
                key={group.id}
                type="button"
                onClick={() => onSelect(group)}
                aria-pressed={selected === group.id}
                aria-label={ui("{value0}, {value1} actor leases, {value2} workers", {
                  value0: group.name,
                  value1: group.leases,
                  value2: group.workers.length,
                })}
                className={cn(
                  "min-w-0 border bg-background p-2 text-left transition-colors hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring",
                  selected === group.id && "border-accent ring-1 ring-accent",
                )}
              >
                <div className="mb-1 flex min-w-0 items-baseline justify-between gap-2">
                  <span className="truncate font-mono text-[10px]">{group.name}</span>
                  <span className="text-[11px] tabular-nums">{number(group.leases)}</span>
                </div>
                <svg viewBox="0 0 120 48" className="w-full" aria-hidden="true">
                  {cellPaths(group.leases, ranges).map(({ color, path }) => (
                    <path key={color} d={path} fill={color} fillOpacity=".7" />
                  ))}
                </svg>
                {group.capacity != null && (
                  <>
                    <div className="mt-1 h-1 bg-muted">
                      <div className="h-full bg-accent" style={{ width: `${load}%` }} />
                    </div>
                    <div className="mt-1 flex justify-between text-[9px] text-muted-foreground">
                      <span>{ui("Reported load")}</span>
                      <span>
                        {group.load}/{group.capacity}
                      </span>
                    </div>
                  </>
                )}
                <div className="mt-1 text-[10px] text-muted-foreground">
                  {group.workers.length} {ui("workers")}
                  {group.stale > 0 && (
                    <span className="ml-2 text-warning">
                      {group.stale} {ui("stale")}
                    </span>
                  )}
                </div>
              </button>
            );
          })}
        </div>
      )}
      <div className="mt-4 flex flex-wrap gap-4 text-xs text-muted-foreground">
        {phases.map((phase) => (
          <span key={phase.key} className="inline-flex items-center gap-1.5">
            <span className="size-2" style={{ background: phase.color }} />
            {phase.label}
          </span>
        ))}
      </div>
      <p className="mt-3 text-xs text-muted-foreground">
        {ui(
          "Cells aggregate leased actors. Reported worker load includes background activities and is separate from held leases. Parked, unleased sessions appear in Execution.",
        )}
      </p>
    </section>
  );
}

function cellPaths(leases: number, ranges: Array<{ color: string; start: number; end: number }>) {
  const paths = new Map<string, string>();
  for (let i = 0; i < 60; i++) {
    const position = ((i + 0.5) / 60) * leases;
    const color =
      (leases ? ranges.find((p) => position >= p.start && position < p.end)?.color : null) ??
      "hsl(var(--muted))";
    paths.set(
      color,
      (paths.get(color) ?? "") + `M${(i % 12) * 10},${Math.floor(i / 12) * 9.5}h8v7.5h-8Z`,
    );
  }
  return [...paths].map(([color, path]) => ({ color, path }));
}
