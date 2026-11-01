"use client";

// Schedules for a session recording. Registered schedules stay listed after
// they are canceled or deleted, so the recording shows what was set up and what
// was removed. Live schedules show their limits (end time, run count, budget)
// and let a person extend them or re-enable one the platform stopped.

import { useLocale as useFormattingLocale, useUiTranslations } from "@/providers/locale-provider";

import { useState } from "react";
import { AlertCircle, CalendarClock, Repeat } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { formatCostUsd, formatRelativeTime } from "@/lib/formatting";
import {
  formatScheduleCadence,
  formatScheduledDateTime,
  getScheduleDisplayTitle,
} from "@/lib/schedule-display";
import type { SessionSchedule } from "@/lib/api/types";
import { useSessionSchedules, useUpdateSessionSchedule } from "@/hooks/use-session-schedules";
import { SessionScheduleExtendDialog } from "./session-schedule-extend-dialog";

type ScheduleStatus = "Active" | "Disabled" | "Deleted" | "Ended" | "Budget used up";

function scheduleStatus(schedule: SessionSchedule): ScheduleStatus {
  if (schedule.deleted_at) return "Deleted";
  if (!schedule.enabled && schedule.stop_reason === "ended") return "Ended";
  if (!schedule.enabled && schedule.stop_reason === "budget_exhausted") return "Budget used up";
  return schedule.enabled ? "Active" : "Disabled";
}

const STATUS_BADGE: Record<ScheduleStatus, "default" | "secondary" | "outline" | "destructive"> = {
  Active: "default",
  Disabled: "secondary",
  Deleted: "secondary",
  Ended: "outline",
  "Budget used up": "destructive",
};

export function SessionScheduleList({ schedules }: { schedules: SessionSchedule[] }) {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  if (schedules.length === 0) {
    return (
      <div className="text-sm text-muted-foreground">{ui("This session scheduled no work.")}</div>
    );
  }

  return (
    <div className="space-y-2">
      {schedules.map((schedule) => {
        const isRecurring = schedule.schedule_type === "recurring";
        const status = scheduleStatus(schedule);
        const cadence = formatScheduleCadence({
          cronExpression: schedule.cron_expression,
          scheduledAt: schedule.scheduled_at,
          timezone: schedule.timezone,
        });

        return (
          <div
            key={schedule.id}
            className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border bg-card px-4 py-3"
          >
            <div className="flex min-w-0 items-center gap-2">
              {isRecurring ? (
                <Repeat className="h-4 w-4 shrink-0 text-muted-foreground" />
              ) : (
                <CalendarClock className="h-4 w-4 shrink-0 text-muted-foreground" />
              )}
              <div className="min-w-0">
                <div className="truncate text-sm font-medium">
                  {getScheduleDisplayTitle(schedule.description)}
                </div>
                <div className="text-xs text-muted-foreground">
                  {cadence}
                  {schedule.next_trigger_at && status === "Active"
                    ? ui(" · next {value0}", {
                        value0: formatScheduledDateTime(
                          schedule.next_trigger_at,
                          schedule.timezone,
                        ),
                      })
                    : ""}
                  {schedule.max_runs != null
                    ? ui(" · Run {value0} of {value1}", {
                        value0: schedule.trigger_count,
                        value1: schedule.max_runs,
                      })
                    : schedule.trigger_count > 0
                      ? ui(" · fired {value0}×", { value0: schedule.trigger_count })
                      : ""}
                  {schedule.ends_at && !schedule.deleted_at
                    ? ui(" · Ends {value0}", {
                        value0: formatScheduledDateTime(schedule.ends_at, schedule.timezone),
                      })
                    : ""}
                  {schedule.budget
                    ? ui(" · {value0} of {value1}", {
                        value0: formatCostUsd(schedule.budget.spent_usd),
                        value1: formatCostUsd(schedule.budget.limit_usd),
                      })
                    : ""}
                  {ui(" · registered {value0}", {
                    value0: formatRelativeTime(schedule.created_at, displayLocale),
                  })}
                  {schedule.deleted_at
                    ? ui(" · deleted {value0}", {
                        value0: formatRelativeTime(schedule.deleted_at, displayLocale),
                      })
                    : ""}
                </div>
                {schedule.stop_when && (
                  <div className="mt-1 text-xs text-muted-foreground">
                    {ui("Stops when: {value0}", { value0: schedule.stop_when })}
                  </div>
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Badge variant={STATUS_BADGE[status]}>{ui(status)}</Badge>
              {hasLimits(schedule) && !schedule.deleted_at && (
                <ScheduleActions schedule={schedule} />
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

/** Only limited schedules can end or run out; an open-ended one has nothing to extend. */
function hasLimits(schedule: SessionSchedule): boolean {
  return (
    schedule.ends_at != null ||
    schedule.max_runs != null ||
    schedule.budget != null ||
    schedule.stop_reason != null
  );
}

function ScheduleActions({ schedule }: { schedule: SessionSchedule }) {
  const ui = useUiTranslations();
  const update = useUpdateSessionSchedule();
  const [extending, setExtending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function reEnable() {
    setError(null);
    try {
      await update.mutateAsync({
        sessionId: schedule.session_id,
        scheduleId: schedule.id,
        request: { enabled: true },
      });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : ui("Could not re-enable the schedule."));
    }
  }

  return (
    <>
      <Button size="sm" variant="outline" onClick={() => setExtending(true)}>
        {ui("Extend")}
      </Button>
      {!schedule.enabled && (
        <Button size="sm" variant="outline" onClick={reEnable} disabled={update.isPending}>
          {ui("Re-enable")}
        </Button>
      )}
      {error && (
        <span role="alert" className="text-xs text-destructive">
          {error}
        </span>
      )}
      {/* Mounted only while open, so each opening starts from the current values. */}
      {extending && (
        <SessionScheduleExtendDialog schedule={schedule} open onOpenChange={setExtending} />
      )}
    </>
  );
}

export function SessionSchedulesSection({
  sessionId,
  announceEmpty,
  showHeading = true,
}: {
  sessionId: string;
  /** When false, an empty list renders nothing instead of the empty sentence. */
  announceEmpty: boolean;
  /** The Files tab already names this pane, so it skips the section heading. */
  showHeading?: boolean;
}) {
  const ui = useUiTranslations();
  const { data: schedules, isLoading, error } = useSessionSchedules(sessionId);

  if (isLoading) return <Skeleton className="h-24 w-full" />;

  if (error) {
    return (
      <div className="flex items-center gap-2 text-destructive">
        <AlertCircle className="h-5 w-5" />
        <span>{error instanceof Error ? error.message : ui("Failed to load schedules")}</span>
      </div>
    );
  }

  const heading = showHeading ? (
    <h2 className="text-sm font-medium text-muted-foreground">{ui("Schedules")}</h2>
  ) : null;

  if (!schedules || schedules.length === 0) {
    if (!announceEmpty) return null;
    return (
      <section className="space-y-3">
        {heading}
        <div className="text-sm text-muted-foreground">{ui("This session scheduled no work.")}</div>
      </section>
    );
  }

  return (
    <section className="space-y-3">
      {heading}
      <SessionScheduleList schedules={schedules} />
    </section>
  );
}
