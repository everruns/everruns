import type { MonitoringSnapshot } from "@/lib/api/types";

export const phases = [
  {
    key: "runnable",
    label: "Runnable",
    color: "hsl(var(--accent))",
    description: "Ready for a worker lease. Expired leases awaiting reacquisition are included.",
  },
  {
    key: "model",
    label: "Model",
    color: "hsl(var(--info))",
    description: "A committed model phase, including request setup and streaming.",
  },
  {
    key: "tools",
    label: "Tools",
    color: "hsl(var(--success))",
    description: "A committed tool execution phase. Parallel calls belong to the same turn.",
  },
  {
    key: "waiting",
    label: "Waiting",
    color: "hsl(var(--muted-foreground))",
    description: "Paused for input or external tool results. Parked actors can be unleased.",
  },
  {
    key: "setup",
    label: "Setup / handoff",
    color: "hsl(var(--muted-foreground))",
    description: "Turn setup or a phase handoff, as observed in the committed event log.",
  },
] as const;
export type PhaseKey = (typeof phases)[number]["key"];
export function phaseCount(data: MonitoringSnapshot, phase: string): number {
  return data.phases.find((p) => p.phase === phase)?.count ?? 0;
}
export const number = (value: number) =>
  new Intl.NumberFormat("en", { maximumFractionDigits: 1 }).format(value);
export function duration(ms: number | null | undefined): string {
  if (ms == null) return "Unavailable";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60000) return `${number(ms / 1000)}s`;
  return `${number(ms / 60000)}m`;
}
export function rate(data: MonitoringSnapshot, event: string): number {
  return (data.events[event] ?? 0) / data.window_seconds;
}

// Replay retains population summaries, not actor records, prompts, or a full
// worker map per frame. A 15-minute window is bounded to 900 compact samples.
export interface HistoryPoint {
  at: string;
  phases: Record<string, number>;
  phaseOldest: Record<string, number>;
  organizations: number;
  workers: number;
  leaseEvents: Record<string, number>;
  actors: number;
  leases: number;
  expired: number;
  oldestRunnable: number | null;
  events: Record<string, number>;
}
export function historyPoint(data: MonitoringSnapshot): HistoryPoint {
  return {
    at: data.generated_at,
    phases: Object.fromEntries(data.phases.map((p) => [p.phase, p.count])),
    phaseOldest: Object.fromEntries(data.phases.map((p) => [p.phase, p.oldest_ms])),
    organizations: data.organizations,
    workers: data.workers.length,
    leaseEvents: data.lease_events,
    actors: data.actors,
    leases: data.actor_leases,
    expired: data.expired_leases,
    oldestRunnable: data.oldest_runnable_ms ?? null,
    events: data.events,
  };
}
export function appendHistory(history: HistoryPoint[], point: HistoryPoint): HistoryPoint[] {
  if (history.at(-1)?.at === point.at) return history;
  const cutoff = Date.parse(point.at) - 15 * 60 * 1000;
  return [...history.filter((p) => Date.parse(p.at) >= cutoff), point].slice(-900);
}
export function replaySnapshot(
  latest: MonitoringSnapshot,
  point: HistoryPoint,
): MonitoringSnapshot {
  return {
    ...latest,
    generated_at: point.at,
    actors: point.actors,
    actor_leases: point.leases,
    expired_leases: point.expired,
    oldest_runnable_ms: point.oldestRunnable,
    events: point.events,
    lease_events: point.leaseEvents,
    organizations: point.organizations,
    cohorts_truncated: false,
    workers_truncated: false,
    phases: phases.map((p) => ({
      phase: p.key,
      count: point.phases[p.key] ?? 0,
      oldest_ms: point.phaseOldest[p.key] ?? 0,
    })),
    workers: [],
    ownership: [],
    cohorts: [],
  };
}

export function perSecond(value: number): string {
  return new Intl.NumberFormat("en", {
    maximumFractionDigits: value > 0 && value < 1 ? 3 : 1,
  }).format(value);
}
