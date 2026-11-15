"use client";

// Monitoring → Cells (platform only): data plane v2 cells as their one-minute
// reports show them (knowledge/operations/monitoring.md). The table is the
// newest report per cell, at most a minute old; a cell silent for three
// minutes is flagged. Selecting a cell shows its history over the range.

import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, Boxes } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  EmptyState,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
} from "@/components/layout/page-layout";
import { TimeSeriesChart, type TimeSeries } from "@/components/charts/time-series-chart";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { getMonitoringCells, type TimelineRange } from "@/lib/api/monitoring";
import type { MonitoringCell, MonitoringCells } from "@/lib/api/types";
import { number } from "./model";
import { axisFormatter, bucketLabel, timelineColors as C } from "./timeline-model";

const RANGES: TimelineRange[] = ["1h", "6h", "24h", "7d"];

function percent(used: number, limit: number | null | undefined): number | null {
  return limit ? Math.round((used / limit) * 1000) / 10 : null;
}

function bytes(v: number): string {
  if (v >= 1024 ** 3) return `${number(Math.round((v / 1024 ** 3) * 10) / 10)} GiB`;
  return `${number(Math.round(v / 1024 ** 2))} MiB`;
}

export function MonitoringCellsView() {
  const ui = useUiTranslations();
  const [range, setRange] = useState<TimelineRange>("6h");
  const [selected, setSelected] = useState<string | null>(null);
  const cells = useQuery({
    queryKey: ["monitoring-cells", range],
    queryFn: () => getMonitoringCells(range),
    refetchInterval: 30_000,
    retry: false,
  });
  const data = cells.data;
  const cell = data?.cells.find((c) => c.cell === selected) ?? data?.cells[0] ?? null;
  return (
    <div className="space-y-4" data-testid="monitoring-cells">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex" role="group" aria-label={ui("Time range")}>
          {RANGES.map((r) => (
            <Button
              key={r}
              variant="outline"
              size="sm"
              aria-pressed={range === r}
              onClick={() => setRange(r)}
              className={`-ml-px first:ml-0 ${range === r ? "relative z-10 border-primary font-semibold" : "text-muted-foreground"}`}
            >
              {r}
            </Button>
          ))}
        </div>
        <span className="text-xs text-muted-foreground">
          {ui("Each cell reports once a minute")}
        </span>
      </div>
      {!data ? (
        cells.error ? (
          <EmptyState
            icon={<AlertTriangle />}
            title={ui("Cells could not load")}
            description={ui("Refresh when the service is available.")}
            action={<Button onClick={() => cells.refetch()}>{ui("Retry")}</Button>}
          />
        ) : (
          <Skeleton className="h-[400px]" />
        )
      ) : data.cells.length === 0 ? (
        <EmptyState
          icon={<Boxes />}
          title={ui("No cells have reported")}
          description={ui(
            "Cells appear here once a data plane v2 cell sends its first minute report. Workers are on the Timeline and Workers tabs.",
          )}
        />
      ) : (
        <PageColumns>
          <PageMain>
            <CellTable data={data} selected={cell?.cell ?? null} onSelect={setSelected} />
          </PageMain>
          <PageRail>{cell && <CellDetail data={data} cell={cell} />}</PageRail>
        </PageColumns>
      )}
    </div>
  );
}

function CellTable({
  data,
  selected,
  onSelect,
}: {
  data: MonitoringCells;
  selected: string | null;
  onSelect: (cell: string) => void;
}) {
  const ui = useUiTranslations();
  const totals = data.cells.reduce(
    (a, c) => ({
      nodes: a.nodes + c.nodes,
      sessions: a.sessions + c.sessions,
      running: a.running + c.running,
      queued: a.queued + c.queued,
    }),
    { nodes: 0, sessions: 0, running: 0, queued: 0 },
  );
  const head = "px-3 py-2 text-left text-[11px] font-medium text-muted-foreground";
  const num = "px-3 py-2 text-right tabular-nums";
  return (
    <section className="overflow-x-auto border bg-card">
      <table className="w-full text-xs">
        <thead className="border-b">
          <tr>
            <th className={head}>{ui("Cell")}</th>
            <th className={`${head} text-right`}>{ui("Nodes")}</th>
            <th className={`${head} text-right`}>{ui("Sessions")}</th>
            <th className={`${head} text-right`}>{ui("Running")}</th>
            <th className={`${head} text-right`}>{ui("Waiting")}</th>
            <th className={`${head} text-right`}>{ui("CPU")}</th>
            <th className={`${head} text-right`}>{ui("Memory")}</th>
            <th className={`${head} text-right`}>{ui("Admit p99")}</th>
            <th className={head}>{ui("Running, range")}</th>
          </tr>
        </thead>
        <tbody>
          {data.cells.map((c) => {
            const cpu = percent(c.cpu_cores, c.cpu_limit_cores);
            const mem = percent(c.memory_bytes, c.memory_limit_bytes);
            const short = c.nodes_answering < c.nodes;
            return (
              <tr
                key={c.cell}
                className={`cursor-pointer border-b last:border-b-0 hover:bg-muted/50 ${selected === c.cell ? "bg-muted" : ""}`}
                onClick={() => onSelect(c.cell)}
                aria-selected={selected === c.cell}
              >
                <td className="px-3 py-2">
                  <button
                    type="button"
                    className="font-mono text-[11px] underline-offset-4 hover:underline"
                    onClick={() => onSelect(c.cell)}
                  >
                    {c.cell}
                  </button>
                  {c.silent && (
                    <span className="ml-2 text-[10px] text-destructive">{ui("silent")}</span>
                  )}
                </td>
                <td className={`${num} ${short ? "text-warning" : ""}`}>
                  {short ? `${c.nodes_answering}/${c.nodes}` : c.nodes}
                </td>
                <td className={num}>{number(c.sessions)}</td>
                <td className={num}>{number(c.running)}</td>
                <td className={`${num} ${c.queued > 0 ? "text-warning" : ""}`}>
                  {number(c.queued)}
                </td>
                <td className={`${num} ${cpu != null && cpu > 90 ? "text-destructive" : ""}`}>
                  {cpu == null ? "—" : `${number(cpu)}%`}
                </td>
                <td className={`${num} ${mem != null && mem > 90 ? "text-destructive" : ""}`}>
                  {mem == null ? bytes(c.memory_bytes) : `${number(mem)}%`}
                </td>
                <td className={num}>{`${number(c.admit_p99_ms)} ms`}</td>
                <td className="px-3 py-2">
                  <Spark cell={c} />
                </td>
              </tr>
            );
          })}
        </tbody>
        <tfoot className="border-t">
          <tr className="font-semibold">
            <td className="px-3 py-2">
              {ui("{value0} cells", { value0: number(data.cells.length) })}
            </td>
            <td className={num}>{number(totals.nodes)}</td>
            <td className={num}>{number(totals.sessions)}</td>
            <td className={num}>{number(totals.running)}</td>
            <td className={num}>{number(totals.queued)}</td>
          </tr>
        </tfoot>
      </table>
    </section>
  );
}

/** Running turns per bucket, scaled to the cell's own peak. */
function Spark({ cell }: { cell: MonitoringCell }) {
  const values = cell.buckets.map((b) => b.running_peak ?? 0);
  const max = Math.max(1, ...values);
  if (values.length < 2) return null;
  const points = values
    .map((v, i) => `${(i / (values.length - 1)) * 100},${20 - (v / max) * 18}`)
    .join(" ");
  return (
    <svg viewBox="0 0 100 20" preserveAspectRatio="none" className="h-5 w-28" aria-hidden>
      <polyline points={points} fill="none" stroke={C.cells} strokeWidth={1.5} />
    </svg>
  );
}

function CellDetail({ data, cell }: { data: MonitoringCells; cell: MonitoringCell }) {
  const ui = useUiTranslations();
  const { displayLocale } = useLocale();
  const rows = useMemo(
    () =>
      cell.buckets.map((b) => ({
        t: Date.parse(b.start),
        sessions: b.sessions_peak ?? null,
        running: b.running_peak ?? null,
        queued: b.queued_peak ?? null,
        cpu: b.cpu_peak_pct ?? null,
        memory: b.memory_peak_pct ?? null,
        completed: b.turns_completed,
        failed: b.turns_failed,
        elections: b.elections,
        moves: b.moves,
      })),
    [cell.buckets],
  );
  const common = {
    rows,
    bucketMs: data.bucket_seconds * 1000,
    formatX: axisFormatter(data.bucket_seconds, displayLocale),
    formatTooltipTime: bucketLabel(displayLocale, data.bucket_seconds),
  };
  const load: TimeSeries[] = [
    { key: "running", label: ui("Running"), color: C.cells, kind: "area" },
    { key: "queued", label: ui("Waiting"), color: C.wait, kind: "line" },
  ];
  const resources: TimeSeries[] = [
    { key: "cpu", label: ui("CPU"), color: C.cpu, kind: "line" },
    { key: "memory", label: ui("Memory"), color: C.memory, kind: "line" },
  ];
  const turns: TimeSeries[] = [
    { key: "completed", label: ui("Completed"), color: C.completed, kind: "bar", stack: "t" },
    { key: "failed", label: ui("Failed"), color: C.failed, kind: "bar", stack: "t" },
  ];
  const elections = cell.buckets.reduce((a, b) => a + b.elections, 0);
  const moves = cell.buckets.reduce((a, b) => a + b.moves, 0);
  const time = new Intl.DateTimeFormat(displayLocale, { hour: "numeric", minute: "2-digit" });
  return (
    <>
      <RailSection label={ui("Cell")}>
        <h2 className="font-mono text-sm font-semibold">{cell.cell}</h2>
        <dl className="mt-3 space-y-2 text-xs">
          <Row label={ui("Data plane")} value={cell.data_plane} />
          <Row
            label={ui("Last report")}
            value={time.format(new Date(cell.reported_at))}
            warn={cell.silent}
          />
          <Row label={ui("Leader")} value={cell.leader ?? "—"} />
          <Row
            label={ui("Nodes answering")}
            value={`${cell.nodes_answering}/${cell.nodes}`}
            warn={cell.nodes_answering < cell.nodes}
          />
          <Row label={ui("Turns on a retry")} value={number(cell.resumed)} />
          <Row label={ui("Log shipping p99")} value={`${number(cell.ship_lag_p99_ms)} ms`} />
          <Row label={ui("Leader changes, range")} value={number(elections)} warn={elections > 0} />
          <Row label={ui("Sessions moved, range")} value={number(moves)} />
        </dl>
      </RailSection>
      <RailSection label={ui("Load")}>
        <TimeSeriesChart
          {...common}
          series={load}
          height={90}
          formatY={(v) => number(v)}
          ariaLabel={ui("Cell load over time")}
        />
      </RailSection>
      <RailSection label={ui("Fullest node")}>
        <TimeSeriesChart
          {...common}
          series={resources}
          height={90}
          formatY={(v) => `${number(v)}%`}
          ariaLabel={ui("Cell CPU and memory over time")}
        />
      </RailSection>
      <RailSection label={ui("Turns")}>
        <TimeSeriesChart
          {...common}
          series={turns}
          height={80}
          ariaLabel={ui("Cell turns over time")}
        />
      </RailSection>
    </>
  );
}

function Row({ label, value, warn = false }: { label: string; value: string; warn?: boolean }) {
  return (
    <div className="flex justify-between gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className={`truncate text-right tabular-nums ${warn ? "text-destructive" : ""}`}>
        {value}
      </dd>
    </div>
  );
}
