"use client";

import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Clock, CheckCircle2, XCircle, AlertTriangle } from "lucide-react";
import { useSessionSchedules } from "@/hooks/use-sessions";
import { useSessionContext } from "../session-context";
import { formatRelativeTime } from "@/lib/formatting";
import type { SessionScheduleStatus } from "@/lib/api/types";

function statusBadge(status: SessionScheduleStatus) {
  switch (status) {
    case "pending":
      return (
        <Badge variant="outline" className="gap-1">
          <Clock className="w-3 h-3" />
          Pending
        </Badge>
      );
    case "triggered":
      return (
        <Badge variant="secondary" className="gap-1">
          <CheckCircle2 className="w-3 h-3" />
          Triggered
        </Badge>
      );
    case "cancelled":
      return (
        <Badge variant="secondary" className="gap-1 opacity-60">
          <XCircle className="w-3 h-3" />
          Cancelled
        </Badge>
      );
    case "failed":
      return (
        <Badge variant="destructive" className="gap-1">
          <AlertTriangle className="w-3 h-3" />
          Failed
        </Badge>
      );
  }
}

export default function SchedulesPage() {
  const { sessionId } = useSessionContext();
  const { data, isLoading } = useSessionSchedules(sessionId);
  const schedules = data?.data ?? [];

  if (isLoading) {
    return (
      <div className="p-4 space-y-3">
        <Skeleton className="h-16 w-full" />
        <Skeleton className="h-16 w-full" />
      </div>
    );
  }

  if (schedules.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-center text-muted-foreground p-8">
        <Clock className="w-12 h-12 mb-4 opacity-50" />
        <p className="text-lg font-medium">No scheduled tasks</p>
        <p className="text-sm">Ask the agent to schedule work for later and it will appear here.</p>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto p-4">
      <div className="space-y-2">
        {schedules.map((schedule) => (
          <div key={schedule.id} className="flex items-start justify-between p-3 rounded-md border">
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <p className="font-medium">{schedule.description}</p>
                {statusBadge(schedule.status)}
              </div>
              <div className="flex items-center gap-3 mt-1 text-xs text-muted-foreground">
                <span>Scheduled for {new Date(schedule.scheduled_at).toLocaleString()}</span>
                <span>Created {formatRelativeTime(schedule.created_at)}</span>
                {schedule.triggered_at && (
                  <span>Triggered {new Date(schedule.triggered_at).toLocaleString()}</span>
                )}
              </div>
              {schedule.error && <p className="text-xs text-destructive mt-1">{schedule.error}</p>}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
