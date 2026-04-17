"use client";

import { useState } from "react";
import {
  useSessionSchedules,
  useUpdateSessionSchedule,
  useDeleteSessionSchedule,
  useTriggerSessionSchedule,
} from "@/hooks/use-session-schedules";
import { useSessionContext } from "../session-context";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Clock, CalendarClock, Repeat, Play, Pause, Trash2, Zap, AlertCircle } from "lucide-react";
import { formatRelativeTime } from "@/lib/formatting";
import {
  formatScheduleCadence,
  formatScheduledDateTime,
  getScheduleDisplayTitle,
} from "@/lib/schedule-display";
import type { SessionSchedule } from "@/lib/api/types";

function ScheduleCard({ schedule, sessionId }: { schedule: SessionSchedule; sessionId: string }) {
  const [deleteOpen, setDeleteOpen] = useState(false);
  const updateMutation = useUpdateSessionSchedule();
  const deleteMutation = useDeleteSessionSchedule();
  const triggerMutation = useTriggerSessionSchedule();

  const isRecurring = schedule.schedule_type === "recurring";
  const scheduleTitle = getScheduleDisplayTitle(schedule.description);
  const scheduleLabel = formatScheduleCadence({
    cronExpression: schedule.cron_expression,
    scheduledAt: schedule.scheduled_at,
    timezone: schedule.timezone,
  });

  const handleToggle = () => {
    updateMutation.mutate({
      sessionId,
      scheduleId: schedule.id,
      request: { enabled: !schedule.enabled },
    });
  };

  const handleDelete = () => {
    deleteMutation.mutate({ sessionId, scheduleId: schedule.id });
    setDeleteOpen(false);
  };

  const handleTrigger = () => {
    triggerMutation.mutate({ sessionId, scheduleId: schedule.id });
  };

  return (
    <>
      <Card className={!schedule.enabled ? "opacity-60" : undefined}>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              {isRecurring ? (
                <Repeat className="h-4 w-4 text-muted-foreground" />
              ) : (
                <CalendarClock className="h-4 w-4 text-muted-foreground" />
              )}
              <CardTitle className="text-base">{scheduleTitle}</CardTitle>
            </div>
            <div className="flex items-center gap-1">
              <Badge variant={schedule.enabled ? "default" : "secondary"}>
                {schedule.enabled ? "Active" : "Disabled"}
              </Badge>
              <Badge variant="outline">{isRecurring ? "Recurring" : "One-shot"}</Badge>
            </div>
          </div>
          {(scheduleLabel || schedule.cron_expression) && (
            <div className="space-y-1">
              {scheduleLabel && <CardDescription>{scheduleLabel}</CardDescription>}
              {schedule.cron_expression && (
                <div className="font-mono text-[11px] text-muted-foreground">
                  cron: {schedule.cron_expression}
                </div>
              )}
            </div>
          )}
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between">
            <div className="space-y-1 text-sm text-muted-foreground">
              {schedule.next_trigger_at && (
                <div className="flex items-center gap-1">
                  <Clock className="h-3 w-3" />
                  <span>
                    Next: {formatScheduledDateTime(schedule.next_trigger_at, schedule.timezone)} (
                    {formatRelativeTime(schedule.next_trigger_at)})
                  </span>
                </div>
              )}
              {schedule.last_triggered_at && (
                <div className="flex items-center gap-1">
                  <Zap className="h-3 w-3" />
                  <span>Last triggered: {formatRelativeTime(schedule.last_triggered_at)}</span>
                </div>
              )}
              {schedule.trigger_count > 0 && (
                <div className="text-xs">
                  Triggered {schedule.trigger_count} time{schedule.trigger_count !== 1 ? "s" : ""}
                </div>
              )}
              <div className="text-xs">Timezone: {schedule.timezone}</div>
            </div>
            <div className="flex items-center gap-1">
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-8 w-8"
                      onClick={handleToggle}
                      disabled={updateMutation.isPending}
                    >
                      {schedule.enabled ? (
                        <Pause className="h-4 w-4" />
                      ) : (
                        <Play className="h-4 w-4" />
                      )}
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>
                    {schedule.enabled ? "Disable schedule" : "Enable schedule"}
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-8 w-8"
                      onClick={handleTrigger}
                      disabled={triggerMutation.isPending || !schedule.enabled}
                    >
                      <Zap className="h-4 w-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Trigger now</TooltipContent>
                </Tooltip>
              </TooltipProvider>
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-8 w-8 text-destructive hover:text-destructive"
                      onClick={() => setDeleteOpen(true)}
                      disabled={deleteMutation.isPending}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Delete schedule</TooltipContent>
                </Tooltip>
              </TooltipProvider>
            </div>
          </div>
        </CardContent>
      </Card>

      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>Delete Schedule</DialogTitle>
            <DialogDescription>
              Are you sure you want to delete this schedule? This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={handleDelete}>
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

export default function SchedulesPage() {
  const { sessionId } = useSessionContext();
  const { data: schedules, isLoading, error } = useSessionSchedules(sessionId);

  if (isLoading) {
    return (
      <div className="flex-1 p-6 space-y-4">
        <Skeleton className="h-32 w-full" />
        <Skeleton className="h-32 w-full" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex-1 p-6">
        <div className="flex items-center gap-2 text-destructive">
          <AlertCircle className="h-5 w-5" />
          <span>{error instanceof Error ? error.message : "Failed to load schedules"}</span>
        </div>
      </div>
    );
  }

  if (!schedules || schedules.length === 0) {
    return (
      <div className="flex-1 p-6">
        <div className="flex flex-col items-center justify-center h-64 text-muted-foreground">
          <CalendarClock className="h-12 w-12 mb-4 opacity-50" />
          <p className="text-lg font-medium">No schedules</p>
          <p className="text-sm mt-1">Ask the agent to schedule work and it will appear here.</p>
        </div>
      </div>
    );
  }

  const active = schedules.filter((s) => s.enabled);
  const inactive = schedules.filter((s) => !s.enabled);

  return (
    <div className="flex-1 p-6 space-y-4 overflow-y-auto">
      {active.length > 0 && (
        <div className="space-y-3">
          <h3 className="text-sm font-medium text-muted-foreground">Active ({active.length})</h3>
          {active.map((schedule) => (
            <ScheduleCard key={schedule.id} schedule={schedule} sessionId={sessionId} />
          ))}
        </div>
      )}
      {inactive.length > 0 && (
        <div className="space-y-3">
          <h3 className="text-sm font-medium text-muted-foreground">
            Inactive ({inactive.length})
          </h3>
          {inactive.map((schedule) => (
            <ScheduleCard key={schedule.id} schedule={schedule} sessionId={sessionId} />
          ))}
        </div>
      )}
    </div>
  );
}
