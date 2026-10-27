"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import cronstrue from "cronstrue";
import { CronExpressionParser } from "cron-parser";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

const CRON_PRESETS = [
  { label: "Every minute", value: "0 * * * * * *" },
  { label: "5 min", value: "0 */5 * * * * *" },
  { label: "Hourly :00", value: "0 0 * * * * *" },
  { label: "Hourly :30", value: "0 30 * * * * *" },
  { label: "Daily 09:00", value: "0 0 9 * * * *" },
];

// The server's `cron` crate numbers 7-field weekdays 1-7 from Sunday, while
// cron-parser and cronstrue read 0-6 from Sunday. Day names mean the same day
// to both, so 7-field weekday numbers become names before previewing (EVE-1260).
const CRATE_WEEKDAYS = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

function crateWeekdaysToNames(field: string): string | null {
  const day = (value: string): string | null => {
    if (!/^\d+$/.test(value)) return value;
    const index = Number(value);
    return index >= 1 && index <= 7 ? CRATE_WEEKDAYS[index - 1] : null;
  };
  const parts = field.split(",").map((part) => {
    const [range, step] = part.split("/");
    const days = range.split("-").map(day);
    if (days.some((value) => value === null)) return null;
    const named = days.join("-");
    return step === undefined ? named : `${named}/${step}`;
  });
  return parts.some((part) => part === null) ? null : parts.join(",");
}

/** The expression as cron-parser and cronstrue read it: 5-field standard cron
 *  as is, or the 7-field crate form (year `*`) as 6 fields with named weekdays. */
function previewExpression(expression: string): string | null {
  const fields = expression.trim().split(/\s+/).filter(Boolean);
  if (fields.length === 5) return fields.join(" ");
  if (fields.length === 7 && fields[6] === "*") {
    const weekdays = crateWeekdaysToNames(fields[5]);
    if (weekdays === null) return null;
    return [...fields.slice(0, 5), weekdays].join(" ");
  }
  return null;
}

export function describeCronExpression(expression: string): string {
  try {
    const parserExpression = previewExpression(expression);
    if (!parserExpression) return "Invalid schedule";
    return cronstrue.toString(parserExpression, {
      throwExceptionOnParseError: true,
      verbose: false,
    });
  } catch {
    return "Invalid schedule";
  }
}

export function isSupportedCronExpression(expression: string): boolean {
  if (describeCronExpression(expression) === "Invalid schedule") return false;
  return getNextRuns(expression, "UTC").length > 0;
}

// Mirror of the server-side SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS default.
export const CRON_MIN_INTERVAL_SECONDS = 300;

/** Returns the minimum interval in seconds across the next 8 runs, or null when
 *  the expression is invalid. Mirrors the server's min-interval check. */
export function getCronIntervalSeconds(expression: string): number | null {
  try {
    const parserExpression = previewExpression(expression);
    if (!parserExpression) return null;
    const iter = CronExpressionParser.parse(parserExpression, { tz: "UTC" });
    const times = Array.from({ length: 9 }, () => iter.next().getTime());
    const deltas = times.slice(1).map((t, i) => Math.round((t - times[i]) / 1000));
    return Math.min(...deltas);
  } catch {
    return null;
  }
}

export function CronLabel({
  expr,
  tz,
  className,
}: {
  expr: string;
  tz?: string;
  className?: string;
}) {
  const timezone = tz?.trim() || "UTC";
  return (
    <span className={className}>
      {describeCronExpression(expr)} · {timezone}
    </span>
  );
}

function getNextRuns(expression: string, timezone: string): Date[] {
  try {
    const parserExpression = previewExpression(expression);
    if (!parserExpression) return [];
    const interval = CronExpressionParser.parse(parserExpression, {
      tz: timezone,
    });
    return Array.from({ length: 8 }, () => interval.next().toDate());
  } catch {
    return [];
  }
}

function formatRunTime(date: Date, timezone: string): string {
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: timezone,
  }).format(date);
}

function formatRunOffset(date: Date): string {
  const minutes = Math.max(0, Math.round((date.getTime() - Date.now()) / 60000));
  if (minutes < 60) return `in ${minutes} min`;
  const hours = Math.round(minutes / 60);
  return `+${hours}h`;
}

export function CronInput({
  value,
  timezone,
  onChange,
  onTimezoneChange,
  className,
}: {
  value: string;
  timezone: string;
  onChange: (value: string) => void;
  onTimezoneChange: (value: string) => void;
  className?: string;
}) {
  const ui = useUiTranslations();
  const resolvedTimezone = timezone || "UTC";
  const description = describeCronExpression(value);
  const nextRuns = getNextRuns(value, resolvedTimezone);
  const invalid = description === "Invalid schedule";

  return (
    <div className={cn("space-y-4", className)}>
      <div className="flex flex-wrap gap-2">
        {CRON_PRESETS.map((preset) => (
          <Button
            key={preset.value}
            type="button"
            variant={preset.value === value ? "default" : "outline"}
            size="sm"
            onClick={() => onChange(preset.value)}
          >
            {preset.label}
          </Button>
        ))}
      </div>

      <div className="grid gap-4 md:grid-cols-[minmax(0,1fr),220px]">
        <div className="space-y-2">
          <Label htmlFor="cron_expression">{ui("Cron expression")}</Label>
          <Input
            id="cron_expression"
            value={value}
            onChange={(event) => onChange(event.target.value)}
            aria-invalid={invalid}
            className="font-mono"
            placeholder="0 30 * * * * *"
          />
          <p className={cn("text-xs", invalid ? "text-destructive" : "text-muted-foreground")}>
            {invalid
              ? ui("Enter a valid 5-field cron, or a 7-field cron with * as the year.")
              : description}
          </p>
        </div>
        <div className="space-y-2">
          <Label htmlFor="cron_timezone">{ui("Timezone")}</Label>
          <Input
            id="cron_timezone"
            value={timezone}
            onChange={(event) => onTimezoneChange(event.target.value)}
            placeholder={ui("UTC")}
          />
        </div>
      </div>

      <div className="space-y-2">
        <p className="text-xs font-medium uppercase text-muted-foreground">{ui("Next 8 runs")}</p>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4 lg:grid-cols-8">
          {nextRuns.length > 0 ? (
            nextRuns.map((run) => (
              <div key={run.toISOString()} className="border bg-background p-2 text-center">
                <p className="text-sm font-semibold">{formatRunTime(run, resolvedTimezone)}</p>
                <p className="text-xs text-muted-foreground">{formatRunOffset(run)}</p>
              </div>
            ))
          ) : (
            <div className="col-span-full border border-dashed p-3 text-sm text-muted-foreground">
              {ui("Next runs appear after the schedule is valid.")}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
