import type { MonitoringSnapshot } from "@/lib/api/types";
import { phases, duration, number, perSecond, type HistoryPoint } from "./model";

export function MonitoringMetrics({
  data,
  history,
}: {
  data: MonitoringSnapshot;
  history: HistoryPoint[];
}) {
  const signals = [
    ["Turns started", "turn.started"],
    ["Model iterations", "reason.started"],
    ["Tool batches", "act.started"],
    ["Tool calls started", "tool.started"],
    ["Tool calls settled", "tool.completed"],
    ["Turns completed", "turn.completed"],
    ["Turns failed", "turn.failed"],
    ["Turns sealed", "turn.sealed"],
    ["Turns cancelled", "turn.cancelled"],
  ];
  return (
    <section className="border bg-card p-4" data-testid="monitoring-metrics">
      <h2 className="text-sm font-semibold">Execution metrics</h2>
      <p className="mt-1 text-xs text-muted-foreground">
        Event counts over the last {data.window_seconds}s. History is collected while Monitoring is
        open.
      </p>
      <table className="mt-4 w-full text-left text-xs">
        <thead className="border-b text-muted-foreground">
          <tr>
            <th className="py-2 font-medium">Signal</th>
            <th className="py-2 text-right font-medium">Count</th>
            <th className="py-2 text-right font-medium">Per second</th>
            <th className="py-2 text-right font-medium">Observed trend</th>
          </tr>
        </thead>
        <tbody>
          {signals.map(([label, event]) => {
            const values = history.slice(-120).map((h) => h.events[event] ?? 0);
            const max = Math.max(1, ...values);
            return (
              <tr key={event} className="border-b border-border/50">
                <td className="py-3">{label}</td>
                <td className="py-3 text-right tabular-nums">{number(data.events[event] ?? 0)}</td>
                <td className="py-3 text-right tabular-nums">
                  {perSecond((data.events[event] ?? 0) / data.window_seconds)}
                </td>
                <td className="py-3 text-right">
                  {values.length > 1 ? (
                    <svg
                      viewBox="0 0 120 24"
                      className="ml-auto h-6 w-28"
                      role="img"
                      aria-label={`${label} recent trend`}
                    >
                      <path
                        d={values
                          .map(
                            (v, i) =>
                              `${i ? "L" : "M"}${(i / (values.length - 1)) * 120},${23 - (v / max) * 22}`,
                          )
                          .join(" ")}
                        stroke="hsl(var(--info))"
                        strokeWidth="1.5"
                        fill="none"
                      />
                    </svg>
                  ) : (
                    <span className="text-muted-foreground">Collecting</span>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <div className="mt-6 grid gap-6 sm:grid-cols-2">
        <div>
          <h3 className="mb-3 text-xs font-semibold">Current population</h3>
          <dl className="space-y-2 text-xs">
            {phases.map((p) => (
              <div key={p.key} className="flex justify-between">
                <dt className="text-muted-foreground">{p.label}</dt>
                <dd className="tabular-nums">
                  {number(data.phases.find((r) => r.phase === p.key)?.count ?? 0)}
                </dd>
              </div>
            ))}
          </dl>
        </div>
        <div>
          <h3 className="mb-3 text-xs font-semibold">Actor leases</h3>
          <dl className="space-y-2 text-xs">
            <div className="flex justify-between">
              <dt className="text-muted-foreground">Held</dt>
              <dd>{number(data.actor_leases)}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-muted-foreground">Expired, not reacquired</dt>
              <dd>{number(data.expired_leases)}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-muted-foreground">Reacquired in window</dt>
              <dd>{number(data.lease_events.reacquired ?? 0)}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-muted-foreground">Oldest runnable</dt>
              <dd>{duration(data.oldest_runnable_ms)}</dd>
            </div>
          </dl>
        </div>
      </div>
    </section>
  );
}
