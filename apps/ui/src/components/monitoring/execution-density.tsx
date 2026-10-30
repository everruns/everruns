"use client";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import type { HistoryPoint } from "./model";
import { phases, number, duration } from "./model";

export function ExecutionDensity({
  history,
  selected,
  onSelect,
}: {
  history: HistoryPoint[];
  selected: number | null;
  onSelect: (index: number) => void;
}) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  // At most 180 visible bins, independent of session population or the number
  // of retained one-second samples. Each bin uses the last observation.
  const stride = Math.max(1, Math.ceil(history.length / 180));
  const bins = history.filter((_, i) => i % stride === 0 || i === history.length - 1);
  const max = Object.fromEntries(
    phases.map((p) => [p.key, Math.max(1, ...history.map((h) => h.phases[p.key] ?? 0))]),
  );
  const oldest = Math.max(1, ...history.map((p) => p.oldestRunnable ?? 0));
  return (
    <section className="border bg-card p-4" data-testid="monitoring-density">
      <div className="flex items-baseline justify-between gap-3">
        <h2 className="text-sm font-semibold">{ui("Execution density over time")}</h2>
        <span className="text-xs text-muted-foreground">
          {ui("History collected while this view is open")}
        </span>
      </div>
      {history.length < 2 ? (
        <p className="py-16 text-center text-sm text-muted-foreground">
          {ui("Collecting live observations. History starts with the first successful snapshot.")}
        </p>
      ) : (
        <div className="mt-4 overflow-x-auto">
          <svg
            viewBox="0 0 960 430"
            className="w-full min-w-[600px]"
            role="group"
            aria-label={ui("Time and execution state density; select a time bucket to replay")}
          >
            {phases.map((phase, row) => (
              <g key={phase.key}>
                <text x="0" y={row * 60 + 22} fontSize="12" fill="hsl(var(--foreground))">
                  {phase.label}
                </text>
                <text x="0" y={row * 60 + 43} fontSize="10" fill="hsl(var(--muted-foreground))">
                  0 – {number(max[phase.key])}
                </text>
                {bins.map((point, index) => {
                  const original = history.indexOf(point);
                  return (
                    <rect
                      key={point.at}
                      x={145 + index * (800 / bins.length)}
                      y={row * 60}
                      width={Math.max(1, 800 / bins.length - 1)}
                      height="51"
                      fill={phase.color}
                      fillOpacity={0.04 + ((point.phases[phase.key] ?? 0) / max[phase.key]) * 0.8}
                      stroke={selected === original ? "hsl(var(--foreground))" : undefined}
                      tabIndex={row === 0 ? 0 : -1}
                      role="button"
                      aria-label={ui("{value0}: {value1} {value2}", {
                        value0: new Date(point.at).toLocaleTimeString(displayLocale),
                        value1: phase.label,
                        value2: point.phases[phase.key] ?? 0,
                      })}
                      onClick={() => onSelect(original)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter" || event.key === " ") {
                          event.preventDefault();
                          onSelect(original);
                        }
                      }}
                      className="cursor-pointer focus:outline-2 focus:outline-ring"
                    >
                      <title>
                        {new Date(point.at).toLocaleTimeString(displayLocale)} · {phase.label}:{" "}
                        {number(point.phases[phase.key] ?? 0)}
                      </title>
                    </rect>
                  );
                })}
              </g>
            ))}
            <text x="145" y="320" fontSize="10" fill="hsl(var(--muted-foreground))">
              {new Date(history[0].at).toLocaleTimeString(displayLocale)}
            </text>
            <text
              x="945"
              y="320"
              textAnchor="end"
              fontSize="10"
              fill="hsl(var(--muted-foreground))"
            >
              {new Date(history.at(-1)!.at).toLocaleTimeString(displayLocale)}
            </text>
            <text x="0" y="366" fontSize="12" fill="hsl(var(--foreground))">
              {ui("Oldest runnable")}
            </text>
            <text x="0" y="385" fontSize="10" fill="hsl(var(--muted-foreground))">
              0 – {duration(oldest)}
            </text>
            <path
              d={bins
                .map(
                  (point, index) =>
                    `${index ? "L" : "M"}${145 + index * (800 / Math.max(1, bins.length - 1))},${410 - ((point.oldestRunnable ?? 0) / oldest) * 65}`,
                )
                .join(" ")}
              fill="none"
              stroke="hsl(var(--accent))"
              strokeWidth="2"
            />
          </svg>
        </div>
      )}
      <p className="mt-3 text-xs text-muted-foreground">
        {ui(
          "Darker bins contain more sessions. Each state has its own scale. Empty history is unavailable, not zero. Select a bucket to replay its population summary.",
        )}
      </p>
    </section>
  );
}
