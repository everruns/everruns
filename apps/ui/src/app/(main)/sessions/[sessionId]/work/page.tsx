"use client";

// Work — the subagents and background work of *this* session (EVE-854). This is
// the retired org-wide `/work` view scoped down to one recording: the same
// `TaskChip` selector and the same `TaskCard` detail, plus the leased-resource
// registry and the session's schedules as read-only facts.
//
// Schedules used to be their own tab with enable/disable/trigger/delete
// controls. A recording cannot be steered, so they land here as inert rows —
// scheduled work is background work.

import { useMemo, useState } from "react";
import Link from "next/link";
import {
  AlertCircle,
  Bot,
  CalendarClock,
  CheckCircle2,
  Clock3,
  Repeat,
  ServerCrash,
  Waypoints,
  Wrench,
  XCircle,
} from "lucide-react";
import { useSessionResources } from "@/hooks/use-session-resources";
import { useSessionSchedules } from "@/hooks/use-session-schedules";
import { useSessionTasks } from "@/hooks/use-session-tasks";
import { TaskChip } from "@/components/session/session-task-chips";
import { TaskCard } from "@/components/session/task-detail";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { formatRelativeTime } from "@/lib/formatting";
import {
  formatScheduleCadence,
  formatScheduledDateTime,
  getScheduleDisplayTitle,
} from "@/lib/schedule-display";
import type { SessionResourceEntry } from "@/lib/api/types";
import { useSessionContext } from "../session-context";

// ============================================
// Tasks
// ============================================

function TasksSection({ sessionId }: { sessionId: string }) {
  const { data: tasks, isLoading, error } = useSessionTasks(sessionId);
  const [selectedId, setSelectedId] = useState<string | undefined>(undefined);

  const ordered = useMemo(
    () => (tasks ?? []).slice().sort((a, b) => b.created_at.localeCompare(a.created_at)),
    [tasks],
  );
  const selected = useMemo(
    () => ordered.find((task) => task.id === selectedId),
    [ordered, selectedId],
  );

  if (isLoading) {
    return <Skeleton className="h-40 w-full" />;
  }

  if (error) {
    return (
      <div className="flex items-center gap-2 text-destructive">
        <AlertCircle className="h-5 w-5" />
        <span>{error instanceof Error ? error.message : "Failed to load tasks"}</span>
      </div>
    );
  }

  if (ordered.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center rounded-lg border border-border bg-card py-12 text-muted-foreground">
        <Waypoints className="mb-4 h-10 w-10 opacity-50" />
        <p className="font-medium">No work yet</p>
        <p className="mt-1 text-sm">
          Subagents, handoffs, background tools and monitors this session started appear here.
        </p>
      </div>
    );
  }

  return (
    <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]">
      <div className="rounded-lg border border-border bg-card p-3">
        <div className="flex flex-wrap items-center gap-1.5">
          {ordered.map((task) => (
            <TaskChip
              key={task.id}
              task={task}
              selected={task.id === selectedId}
              onClick={() => setSelectedId(task.id)}
            />
          ))}
        </div>
      </div>
      <div className="lg:sticky lg:top-6 lg:self-start">
        {selected ? (
          <TaskCard task={selected} sessionId={selected.session_id} />
        ) : (
          <div className="flex h-40 items-center justify-center rounded-lg border border-dashed border-border text-sm text-muted-foreground">
            Select a task to see its detail.
          </div>
        )}
      </div>
    </div>
  );
}

// ============================================
// Leased resources
// ============================================

function statusBadge(status: string) {
  switch (status) {
    case "active":
      return <Badge variant="default">Active</Badge>;
    case "completed":
      return <Badge variant="outline">Completed</Badge>;
    case "failed":
      return <Badge variant="destructive">Failed</Badge>;
    case "released":
      return <Badge variant="secondary">Released</Badge>;
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

function statusIcon(status: string) {
  switch (status) {
    case "active":
      return <Clock3 className="h-4 w-4 text-muted-foreground" />;
    case "completed":
      return <CheckCircle2 className="h-4 w-4 text-muted-foreground" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-destructive" />;
    case "released":
      return <ServerCrash className="h-4 w-4 text-muted-foreground" />;
    default:
      return <Wrench className="h-4 w-4 text-muted-foreground" />;
  }
}

function ResourceCard({ resource }: { resource: SessionResourceEntry }) {
  const statusText =
    typeof resource.metadata?.status_text === "string" ? resource.metadata.status_text : null;
  const summary = typeof resource.metadata?.summary === "string" ? resource.metadata.summary : null;
  const logPath =
    typeof resource.metadata?.log_path === "string" ? resource.metadata.log_path : null;
  const resultPath =
    typeof resource.metadata?.result_path === "string" ? resource.metadata.result_path : null;
  const outputTail =
    typeof resource.metadata?.output_tail === "string" ? resource.metadata.output_tail : null;
  const progress =
    resource.metadata?.progress && typeof resource.metadata.progress === "object"
      ? (resource.metadata.progress as Record<string, unknown>)
      : null;
  const progressLine =
    progress && (progress.current !== undefined || progress.total !== undefined)
      ? `${progress.label ? `${progress.label}: ` : ""}${progress.current ?? "?"}/${progress.total ?? "?"}${typeof progress.unit === "string" ? ` ${progress.unit}` : ""}`
      : null;

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            {statusIcon(resource.status)}
            <div>
              <CardTitle className="text-base">
                <EntityIdentity value={resource.resource_id}>
                  {resource.display_name || resource.resource_id}
                </EntityIdentity>
              </CardTitle>
              <CardDescription>{resource.kind}</CardDescription>
            </div>
          </div>
          {statusBadge(resource.status)}
        </div>
      </CardHeader>
      <CardContent className="space-y-2 text-sm text-muted-foreground">
        <div className="grid gap-1">
          <div>Registered: {formatRelativeTime(resource.created_at)}</div>
          {statusText ? <div>Status: {statusText}</div> : null}
          {progressLine ? <div>Progress: {progressLine}</div> : null}
          {summary ? <div className="text-foreground">Summary: {summary}</div> : null}
          {logPath ? <div className="truncate">Log: {logPath}</div> : null}
          {resultPath ? <div className="truncate">Result: {resultPath}</div> : null}
        </div>
        {outputTail ? (
          <pre className="overflow-x-auto whitespace-pre-wrap rounded border border-border/70 bg-card px-3 py-2 text-xs text-foreground">
            {outputTail}
          </pre>
        ) : null}
      </CardContent>
    </Card>
  );
}

function ResourcesSection({ sessionId }: { sessionId: string }) {
  const { data: resources, isLoading, error } = useSessionResources(sessionId);

  if (isLoading) return <Skeleton className="h-32 w-full" />;

  if (error) {
    return (
      <div className="flex items-center gap-2 text-destructive">
        <AlertCircle className="h-5 w-5" />
        <span>{error instanceof Error ? error.message : "Failed to load resources"}</span>
      </div>
    );
  }

  if (!resources || resources.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center rounded-lg border border-border bg-card py-10 text-muted-foreground">
        <Bot className="mb-3 h-10 w-10 opacity-50" />
        <p className="font-medium">No resources leased</p>
        <p className="mt-1 text-sm">Sandboxes, browser sessions and similar leases appear here.</p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {resources.map((resource) => (
        <ResourceCard key={resource.resource_id} resource={resource} />
      ))}
    </div>
  );
}

// ============================================
// Schedules (read-only)
// ============================================

function SchedulesSection({ sessionId }: { sessionId: string }) {
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

  if (!schedules || schedules.length === 0) {
    return <div className="text-sm text-muted-foreground">This session scheduled no work.</div>;
  }

  return (
    <div className="space-y-2">
      {schedules.map((schedule) => {
        const isRecurring = schedule.schedule_type === "recurring";
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
                  {schedule.next_trigger_at
                    ? ` · next ${formatScheduledDateTime(schedule.next_trigger_at, schedule.timezone)}`
                    : ""}
                  {schedule.trigger_count > 0 ? ` · fired ${schedule.trigger_count}×` : ""}
                </div>
              </div>
            </div>
            <Badge variant={schedule.enabled ? "default" : "secondary"}>
              {schedule.enabled ? "Active" : "Disabled"}
            </Badge>
          </div>
        );
      })}
    </div>
  );
}

export default function SessionWorkPage() {
  const { sessionId, session } = useSessionContext();
  const features = new Set(session?.features ?? []);

  return (
    <div className="flex-1 space-y-8 overflow-y-auto p-6">
      <section className="space-y-3">
        <h2 className="text-sm font-medium text-muted-foreground">Tasks</h2>
        <TasksSection sessionId={sessionId} />
      </section>

      <section className="space-y-3">
        <h2 className="text-sm font-medium text-muted-foreground">Resources</h2>
        <ResourcesSection sessionId={sessionId} />
      </section>

      {features.has("schedules") && (
        <section className="space-y-3">
          <h2 className="text-sm font-medium text-muted-foreground">Schedules</h2>
          <SchedulesSection sessionId={sessionId} />
        </section>
      )}

      <p className="text-xs text-muted-foreground">
        Work across every session used to live at <span className="font-mono">/work</span>; it is
        now scoped to the session that started it. Subagent tasks link to their own recording from
        the detail card, or browse{" "}
        <Link href="/sessions" className="underline">
          all sessions
        </Link>
        .
      </p>
    </div>
  );
}
