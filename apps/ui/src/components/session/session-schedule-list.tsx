"use client";

// Read-only schedules for a session recording. Registered schedules stay
// listed after they are canceled or deleted, so the recording shows what was
// set up and what was removed.

import { AlertCircle, CalendarClock, Repeat } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { formatRelativeTime } from "@/lib/formatting";
import {
  formatScheduleCadence,
  formatScheduledDateTime,
  getScheduleDisplayTitle,
} from "@/lib/schedule-display";
import type { SessionSchedule } from "@/lib/api/types";
import { useSessionSchedules } from "@/hooks/use-session-schedules";

function scheduleStatus(schedule: SessionSchedule): "Active" | "Disabled" | "Deleted" {
  if (schedule.deleted_at) return "Deleted";
  return schedule.enabled ? "Active" : "Disabled";
}

export function SessionScheduleList({ schedules }: { schedules: SessionSchedule[] }) {
  if (schedules.length === 0) {
    return <div className="text-sm text-muted-foreground">This session scheduled no work.</div>;
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
                    ? ` · next ${formatScheduledDateTime(schedule.next_trigger_at, schedule.timezone)}`
                    : ""}
                  {schedule.trigger_count > 0 ? ` · fired ${schedule.trigger_count}×` : ""}
                  {` · registered ${formatRelativeTime(schedule.created_at)}`}
                  {schedule.deleted_at
                    ? ` · deleted ${formatRelativeTime(schedule.deleted_at)}`
                    : ""}
                </div>
              </div>
            </div>
            <Badge variant={status === "Active" ? "default" : "secondary"}>{status}</Badge>
          </div>
        );
      })}
    </div>
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
  const { data: schedules, isLoading, error } = useSessionSchedules(sessionId);

  if (isLoading) return <Skeleton className="h-24 w-full" />;

  if (error) {
    return (
      <div className="flex items-center gap-2 text-destructive">
        <AlertCircle className="h-5 w-5" />
        <span>{error instanceof Error ? error.message : "Failed to load schedules"}</span>
      </div>
    );
  }

  const heading = showHeading ? (
    <h2 className="text-sm font-medium text-muted-foreground">Schedules</h2>
  ) : null;

  if (!schedules || schedules.length === 0) {
    if (!announceEmpty) return null;
    return (
      <section className="space-y-3">
        {heading}
        <div className="text-sm text-muted-foreground">This session scheduled no work.</div>
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
