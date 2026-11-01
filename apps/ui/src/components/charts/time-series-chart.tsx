"use client";

// Shared time-series chart (Slate): flat, zero radius, mono axis labels, and
// colors only from tokens. Charts with the same `syncId` share a crosshair.
// A click selects one bucket; a drag selects a range. Selection is reported as
// bucket indices so the caller owns what a bucket means.

import { useState } from "react";
import {
  Area,
  Bar,
  CartesianGrid,
  ComposedChart,
  Line,
  ReferenceArea,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

export interface TimeSeries {
  key: string;
  label: string;
  color: string;
  kind: "area" | "bar" | "line";
  /** Series sharing a stack id are stacked. */
  stack?: string;
  dashed?: boolean;
}

export interface TimeMarker {
  at: number;
  label: string;
}

export type TimeRow = { t: number } & Record<string, number | null>;

export interface TimeSelection {
  start: number;
  end: number;
}

interface Props {
  rows: TimeRow[];
  series: TimeSeries[];
  height?: number;
  syncId?: string;
  bucketMs: number;
  formatX: (t: number) => string;
  formatY?: (v: number) => string;
  formatValue?: (v: number) => string;
  formatTooltipTime?: (t: number) => string;
  markers?: TimeMarker[];
  selection?: TimeSelection | null;
  onSelect?: (selection: TimeSelection) => void;
  ariaLabel: string;
}

const AXIS = {
  fontSize: 10,
  fontFamily: "var(--font-mono, ui-monospace, monospace)",
  fill: "hsl(var(--muted-foreground))",
};

function index(state: unknown): number | null {
  const value = (state as { activeTooltipIndex?: number | string | null } | null)
    ?.activeTooltipIndex;
  if (value === null || value === undefined || value === "") return null;
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

export function TimeSeriesChart({
  rows,
  series,
  height = 200,
  syncId,
  bucketMs,
  formatX,
  formatY = (v) => String(v),
  formatValue = formatY,
  formatTooltipTime = formatX,
  markers = [],
  selection,
  onSelect,
  ariaLabel,
}: Props) {
  const [drag, setDrag] = useState<TimeSelection | null>(null);
  const finish = () => {
    if (drag && onSelect)
      onSelect({ start: Math.min(drag.start, drag.end), end: Math.max(drag.start, drag.end) });
    setDrag(null);
  };
  const shown = drag ?? selection;
  const at = (i: number) => rows[Math.max(0, Math.min(rows.length - 1, i))]?.t ?? 0;
  return (
    <div role="img" aria-label={ariaLabel} style={{ height }} className="select-none">
      <ResponsiveContainer width="100%" height="100%">
        <ComposedChart
          data={rows}
          syncId={syncId}
          margin={{ top: 6, right: 8, bottom: 0, left: 0 }}
          barCategoryGap={1}
          onMouseDown={(state) => {
            const i = index(state);
            if (i !== null && onSelect) setDrag({ start: i, end: i });
          }}
          onMouseMove={(state) => {
            const i = index(state);
            if (drag && i !== null) setDrag({ ...drag, end: i });
          }}
          onMouseUp={finish}
          onMouseLeave={() => drag && finish()}
        >
          <CartesianGrid vertical={false} stroke="hsl(var(--border))" strokeOpacity={0.6} />
          <XAxis
            dataKey="t"
            type="number"
            scale="time"
            domain={[rows[0]?.t ?? 0, (rows.at(-1)?.t ?? 0) + bucketMs]}
            tickFormatter={formatX}
            tick={AXIS}
            tickLine={false}
            axisLine={{ stroke: "hsl(var(--border))" }}
            minTickGap={48}
          />
          <YAxis
            tickFormatter={formatY}
            tick={AXIS}
            tickLine={false}
            axisLine={false}
            width={44}
            allowDecimals={false}
          />
          <Tooltip
            cursor={{ stroke: "hsl(var(--primary))", strokeOpacity: 0.4 }}
            isAnimationActive={false}
            content={({ active, payload, label }) =>
              active && payload?.length ? (
                <div className="border bg-popover px-3 py-2 text-xs shadow-md">
                  <div className="mb-1 font-semibold">{formatTooltipTime(Number(label))}</div>
                  {series.map((s) => {
                    const value = payload.find((p) => p.dataKey === s.key)?.value;
                    if (value === undefined || value === null) return null;
                    return (
                      <div key={s.key} className="flex justify-between gap-6">
                        <span className="text-muted-foreground">
                          <i
                            className="mr-1.5 inline-block size-2 align-[-1px]"
                            style={{ background: s.color }}
                          />
                          {s.label}
                        </span>
                        <span className="tabular-nums">{formatValue(Number(value))}</span>
                      </div>
                    );
                  })}
                </div>
              ) : null
            }
          />
          {series.map((s) =>
            s.kind === "area" ? (
              <Area
                key={s.key}
                dataKey={s.key}
                name={s.label}
                stackId={s.stack}
                type="linear"
                stroke={s.color}
                strokeWidth={1}
                fill={s.color}
                fillOpacity={0.28}
                isAnimationActive={false}
                connectNulls
              />
            ) : s.kind === "bar" ? (
              <Bar
                key={s.key}
                dataKey={s.key}
                name={s.label}
                stackId={s.stack}
                fill={s.color}
                isAnimationActive={false}
              />
            ) : (
              <Line
                key={s.key}
                dataKey={s.key}
                name={s.label}
                type="linear"
                stroke={s.color}
                strokeWidth={s.dashed ? 1 : 1.5}
                strokeDasharray={s.dashed ? "3 3" : undefined}
                dot={false}
                isAnimationActive={false}
                connectNulls
              />
            ),
          )}
          {markers.map((m) => (
            <ReferenceLine
              key={`${m.at}-${m.label}`}
              x={m.at}
              stroke="hsl(var(--muted-foreground))"
              strokeDasharray="2 3"
              label={{ value: m.label, position: "insideTopLeft", ...AXIS }}
            />
          ))}
          {shown && rows.length > 0 && (
            <ReferenceArea
              x1={at(shown.start)}
              x2={at(shown.end) + bucketMs}
              fill="hsl(var(--accent))"
              fillOpacity={0.1}
              stroke="hsl(var(--accent))"
              strokeOpacity={0.5}
            />
          )}
        </ComposedChart>
      </ResponsiveContainer>
    </div>
  );
}

export function ChartLegend({ series }: { series: Pick<TimeSeries, "key" | "label" | "color">[] }) {
  return (
    <div className="flex flex-wrap justify-end gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
      {series.map((s) => (
        <span key={s.key}>
          <i className="mr-1.5 inline-block size-2 align-[-1px]" style={{ background: s.color }} />
          {s.label}
        </span>
      ))}
    </div>
  );
}
