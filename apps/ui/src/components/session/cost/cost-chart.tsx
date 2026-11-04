"use client";

// Spend over time: per day in the viewer's timezone (rolled up from the UTC
// hours the server returns) or per turn. Bars are plain HTML so the chart
// reflows with the card; hovering a bar shows its breakdown.

import { useMemo, useState } from "react";
import type { CostHour, CostTurn, SessionCost } from "@/lib/api/types";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { useLocale, useUiTranslations } from "@/providers/locale-provider";
import { cn } from "@/lib/utils";
import {
  CATEGORY_BG,
  CATEGORY_ORDER,
  type CostCategoryKey,
  CostCard,
  Swatch,
  formatUsd,
} from "./cost-shared";

/** Days shown at most; a longer session shows its latest days. */
const MAX_DAYS = 92;
const DAY_MS = 86_400_000;

type Mode = "day" | "turn";

interface DayBucket {
  key: string;
  date: Date;
  usd: Record<CostCategoryKey | "unattributed", number>;
  calls: number;
}

function emptyUsd(): DayBucket["usd"] {
  return { uncached_input: 0, cache_read: 0, cache_write: 0, output: 0, unattributed: 0 };
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

/** Roll UTC hours up into local days, filling days without calls. */
export function dayBuckets(hours: CostHour[]): DayBucket[] {
  if (hours.length === 0) return [];
  const byDay = new Map<string, DayBucket>();
  for (const hour of hours) {
    const at = new Date(hour.hour);
    const date = new Date(at.getFullYear(), at.getMonth(), at.getDate());
    const key = dayKey(date);
    const bucket = byDay.get(key) ?? { key, date, usd: emptyUsd(), calls: 0 };
    bucket.usd.uncached_input += hour.uncached_input_usd;
    bucket.usd.cache_read += hour.cache_read_usd;
    bucket.usd.cache_write += hour.cache_write_usd;
    bucket.usd.output += hour.output_usd;
    bucket.usd.unattributed += hour.unattributed_usd;
    bucket.calls += hour.model_calls;
    byDay.set(key, bucket);
  }
  const days = [...byDay.values()].sort((a, b) => a.date.getTime() - b.date.getTime());
  const filled: DayBucket[] = [];
  const last = days[days.length - 1].date;
  // Step by calendar day, not 24h, so a DST change does not skip or repeat one.
  for (
    let d = new Date(days[0].date);
    d.getTime() <= last.getTime() + DAY_MS / 2;
    d = new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1)
  ) {
    const key = dayKey(d);
    filled.push(byDay.get(key) ?? { key, date: new Date(d), usd: emptyUsd(), calls: 0 });
  }
  return filled.slice(-MAX_DAYS);
}

function bucketTotal(bucket: DayBucket): number {
  return Object.values(bucket.usd).reduce((sum, v) => sum + v, 0);
}

/** A round axis maximum above `value`: 1, 2, 2.5 or 5 times a power of ten. */
export function niceMax(value: number): number {
  if (value <= 0) return 1;
  const power = 10 ** Math.floor(Math.log10(value));
  for (const step of [1, 2, 2.5, 5, 10]) {
    if (step * power >= value) return step * power;
  }
  return 10 * power;
}

export function CostChart({ cost }: { cost: SessionCost }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const days = useMemo(() => dayBuckets(cost.hours), [cost.hours]);
  const [mode, setMode] = useState<Mode | null>(null);
  // A session that ran within a day or two reads better turn by turn.
  const effective: Mode = mode ?? (days.length > 2 ? "day" : "turn");

  const range =
    days.length > 0
      ? `${days[0].date.toLocaleDateString(locale, { month: "short", day: "numeric" })} – ${days[
          days.length - 1
        ].date.toLocaleDateString(locale, { month: "short", day: "numeric" })}`
      : "";

  return (
    <CostCard
      title={effective === "day" ? ui("Spend per day") : ui("Spend per turn")}
      meta={
        effective === "day"
          ? ui("{value0} · your timezone", { value0: range })
          : cost.turns_truncated
            ? ui("Latest {value0} turns", { value0: cost.turns.length.toLocaleString(locale) })
            : ui("{value0} turns", { value0: cost.turns.length.toLocaleString(locale) })
      }
      actions={
        <SegmentedControl
          label={ui("Group spend by")}
          value={effective}
          onValueChange={(v) => setMode(v as Mode)}
          options={[
            { value: "day", label: ui("Day") },
            { value: "turn", label: ui("Turn") },
          ]}
        />
      }
    >
      {effective === "day" ? <DayBars days={days} /> : <TurnBars turns={cost.turns} />}
    </CostCard>
  );
}

function Axis({ max, locale }: { max: number; locale: string }) {
  return (
    <div className="pointer-events-none absolute inset-0" aria-hidden>
      {[0, 0.5, 1].map((f) => (
        <div
          key={f}
          className="absolute right-0 left-0 border-t border-border/70"
          style={{ bottom: `${f * 100}%` }}
        >
          <span className="absolute -top-2 -left-14 w-12 text-right text-[11px] text-muted-foreground tabular-nums">
            {formatUsd(max * f, locale).replace(/\.00$/, "")}
          </span>
        </div>
      ))}
    </div>
  );
}

function DayBars({ days }: { days: DayBucket[] }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const [hover, setHover] = useState<number | null>(null);
  const max = niceMax(Math.max(0, ...days.map(bucketTotal)));
  const labelEvery = Math.max(1, Math.ceil(days.length / 14));
  const labels: Record<CostCategoryKey | "unattributed", string> = {
    uncached_input: ui("Prompt, uncached"),
    cache_read: ui("Read from cache"),
    cache_write: ui("Written to cache"),
    output: ui("Output"),
    unattributed: ui("Not split by category"),
  };
  const shown = [...CATEGORY_ORDER, "unattributed" as const].filter((k) =>
    days.some((d) => d.usd[k] > 0),
  );
  const active = hover != null ? days[hover] : null;

  if (days.length === 0) {
    return <p className="p-4 text-sm text-muted-foreground">{ui("No model calls yet.")}</p>;
  }

  return (
    <div className="px-4 pt-6 pb-3">
      <div className="relative ml-14 h-48">
        <Axis max={max} locale={locale} />
        <div className="absolute inset-0 flex items-end" onMouseLeave={() => setHover(null)}>
          {days.map((day, i) => {
            const total = bucketTotal(day);
            return (
              <div
                key={day.key}
                className={cn(
                  "flex h-full flex-1 flex-col justify-end px-[12%]",
                  hover === i && "bg-muted/70",
                )}
                onMouseEnter={() => setHover(i)}
              >
                <div
                  className="flex flex-col-reverse gap-px"
                  style={{ height: `${(total / max) * 100}%` }}
                  role="img"
                  aria-label={`${day.date.toLocaleDateString(locale)}: ${formatUsd(total, locale)}`}
                >
                  {[...CATEGORY_ORDER, "unattributed" as const].map((k) =>
                    day.usd[k] > 0 ? (
                      <span
                        key={k}
                        className={CATEGORY_BG[k]}
                        style={{ flexGrow: day.usd[k], minHeight: 1 }}
                      />
                    ) : null,
                  )}
                </div>
              </div>
            );
          })}
        </div>
        {active ? (
          <div
            className="absolute top-0 z-10 w-56 border border-border bg-popover p-3 text-xs shadow-md"
            style={
              hover! > days.length / 2
                ? { right: `${((days.length - hover!) / days.length) * 100}%` }
                : { left: `${((hover! + 1) / days.length) * 100}%` }
            }
          >
            <div className="mb-1.5 font-semibold">
              {active.date.toLocaleDateString(locale, {
                weekday: "short",
                month: "short",
                day: "numeric",
              })}
            </div>
            {shown.map((k) => (
              <div key={k} className="flex justify-between gap-3 py-0.5">
                <span className="flex items-center gap-1.5">
                  <Swatch className={CATEGORY_BG[k]} />
                  {labels[k]}
                </span>
                <span className="tabular-nums">{formatUsd(active.usd[k], locale)}</span>
              </div>
            ))}
            <div className="mt-1.5 flex justify-between border-t border-border pt-1.5 text-muted-foreground">
              <span>
                {ui("{value0} model calls", { value0: active.calls.toLocaleString(locale) })}
              </span>
              <span className="tabular-nums">{formatUsd(bucketTotal(active), locale)}</span>
            </div>
          </div>
        ) : null}
      </div>
      <div className="ml-14 flex pt-1.5">
        {days.map((day, i) => (
          <span key={day.key} className="flex-1 text-center text-[11px] text-muted-foreground">
            {i % labelEvery === 0
              ? day.date.toLocaleDateString(locale, { month: "short", day: "numeric" })
              : ""}
          </span>
        ))}
      </div>
      <div className="mt-3 flex flex-wrap gap-4 text-xs text-muted-foreground">
        {shown.map((k) => (
          <span key={k} className="flex items-center gap-1.5">
            <Swatch className={CATEGORY_BG[k]} />
            {labels[k]}
          </span>
        ))}
      </div>
    </div>
  );
}

function TurnBars({ turns }: { turns: CostTurn[] }) {
  const { displayLocale: locale } = useLocale();
  const ui = useUiTranslations();
  const [hover, setHover] = useState<number | null>(null);
  const max = niceMax(Math.max(0, ...turns.map((t) => t.usd)));
  const failed = turns.filter((t) => t.status === "failed").length;
  const active = hover != null ? turns[hover] : null;

  if (turns.length === 0) {
    return <p className="p-4 text-sm text-muted-foreground">{ui("No turns yet.")}</p>;
  }
  const dense = turns.length > 200;

  return (
    <div className="px-4 pt-6 pb-3">
      <div className="relative ml-14 h-48">
        <Axis max={max} locale={locale} />
        <div
          className={cn("absolute inset-0 flex items-end", !dense && "gap-px")}
          onMouseLeave={() => setHover(null)}
        >
          {turns.map((turn, i) => (
            <div
              key={turn.turn_id}
              className={cn("flex h-full min-w-0 flex-1 items-end", hover === i && "bg-muted/70")}
              onMouseEnter={() => setHover(i)}
            >
              <span
                className={cn(
                  "block w-full",
                  CATEGORY_BG.uncached_input,
                  hover === i && "opacity-80",
                )}
                style={{ height: `${(turn.usd / max) * 100}%` }}
              />
            </div>
          ))}
        </div>
        {active ? (
          <div
            className="absolute top-0 z-10 w-60 border border-border bg-popover p-3 text-xs shadow-md"
            style={
              hover! > turns.length / 2
                ? { right: `${((turns.length - hover!) / turns.length) * 100}%` }
                : { left: `${((hover! + 1) / turns.length) * 100}%` }
            }
          >
            <div className="mb-1 font-semibold">
              {ui("Turn {value0}", { value0: active.turn.toLocaleString(locale) })}
            </div>
            <div className="text-muted-foreground">
              {new Date(active.started_at).toLocaleString(locale, {
                month: "short",
                day: "numeric",
                hour: "numeric",
                minute: "2-digit",
              })}
            </div>
            <div className="mt-1.5 flex justify-between">
              <span>{ui("Spend")}</span>
              <span className="tabular-nums">{formatUsd(active.usd, locale)}</span>
            </div>
            <div className="flex justify-between">
              <span>{ui("Prompt tokens")}</span>
              <span className="tabular-nums">{active.prompt_tokens.toLocaleString(locale)}</span>
            </div>
            <div className="flex justify-between">
              <span>{ui("Status")}</span>
              <span className={active.status === "failed" ? "text-destructive" : undefined}>
                {active.status}
              </span>
            </div>
          </div>
        ) : null}
      </div>
      {failed > 0 ? (
        <div className="mt-1.5 ml-14 flex h-2" aria-hidden>
          {turns.map((turn) => (
            <span
              key={turn.turn_id}
              className={cn("min-w-0 flex-1", turn.status === "failed" && "bg-destructive")}
            />
          ))}
        </div>
      ) : null}
      <div className="mt-1.5 ml-14 flex justify-between text-[11px] text-muted-foreground">
        <span>{ui("Turn {value0}", { value0: turns[0].turn.toLocaleString(locale) })}</span>
        <span>{turns[turns.length - 1].turn.toLocaleString(locale)}</span>
      </div>
      {failed > 0 ? (
        <div className="mt-3 flex items-center gap-1.5 text-xs text-muted-foreground">
          <Swatch className="bg-destructive" />
          {ui("{value0} failed turns", { value0: failed.toLocaleString(locale) })}
        </div>
      ) : null}
    </div>
  );
}
