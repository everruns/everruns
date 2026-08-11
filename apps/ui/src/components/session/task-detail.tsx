"use client";

// Per-session task detail card (knowledge/runtime-resources/session-tasks.md) — extracted from the
// session Resources page so both the per-session Tasks tab and the
// cross-session Work view (EVE-756) render the identical detail surface:
// state badge, progress, summary/error, result + artifact links, the pending
// input prompt, and the lazily-loaded lifecycle timeline + message thread.
//
// Read-only since EVE-854: a session is a recording, so the card reports what a
// task did and asked for but cannot cancel it or answer it. Both of those are
// reached by forking the session into a chat.
//
// Task reads are keyed by the task's owning session, so callers pass
// `sessionId` (the task's `session_id`).

import { useState } from "react";
import Link from "next/link";
import { ChevronDown, ChevronRight, ListTodo, MessageSquare } from "lucide-react";
import { useSessionTask } from "@/hooks/use-session-tasks";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { formatRelativeTime } from "@/lib/formatting";
import { isTerminalTaskState } from "@/lib/api/types";
import type { SessionTask, SessionTaskState, TaskMessage } from "@/lib/api/types";

export function taskStateBadge(state: SessionTaskState) {
  switch (state) {
    case "queued":
      return <Badge variant="outline">Queued</Badge>;
    case "running":
      return <Badge variant="default">Running</Badge>;
    case "awaiting_input":
      return <Badge className="border-transparent bg-amber-500 text-white">Awaiting input</Badge>;
    case "succeeded":
      return <Badge className="border-transparent bg-green-600 text-white">Succeeded</Badge>;
    case "failed":
      return <Badge variant="destructive">Failed</Badge>;
    case "canceled":
      return <Badge variant="secondary">Canceled</Badge>;
    default:
      return <Badge variant="outline">{state}</Badge>;
  }
}

export function TaskProgressBar({ task }: { task: SessionTask }) {
  const progress = task.progress;
  if (!progress || progress.current == null || progress.total == null || progress.total <= 0) {
    return null;
  }
  const percent = Math.min(100, Math.max(0, (progress.current / progress.total) * 100));
  const line = `${progress.label ? `${progress.label}: ` : ""}${progress.current}/${progress.total}${progress.unit ? ` ${progress.unit}` : ""}`;
  return (
    <div className="space-y-1">
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
        <div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} />
      </div>
      <div className="text-xs">{line}</div>
    </div>
  );
}

/**
 * A task that is waiting on input shows what it asked for, but the answer is
 * not typed here: session detail is a read-only recording (EVE-854), so the
 * prompt is a fact. Fork the session into a chat to actually answer it.
 */
function TaskInputPrompt({ task }: { task: SessionTask }) {
  if (task.state !== "awaiting_input" || !task.input_request) return null;

  return (
    <div className="space-y-1 rounded border border-amber-500/50 bg-amber-500/10 px-3 py-2">
      <div className="text-sm text-foreground">{task.input_request.prompt}</div>
      <div className="text-xs text-muted-foreground">
        Waiting for input. Fork this session into a chat to answer.
      </div>
    </div>
  );
}

/** One message in a task's bidirectional thread. */
function TaskMessageRow({ message }: { message: TaskMessage }) {
  const inbound = message.direction === "inbound";
  return (
    <div className={`flex ${inbound ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[85%] rounded-lg px-3 py-1.5 ${
          inbound ? "bg-primary/10 text-foreground" : "bg-muted text-foreground"
        }`}
      >
        <div className="mb-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
          {inbound ? "Session → task" : "Task → session"} · {formatRelativeTime(message.created_at)}
        </div>
        {message.content.map((part, index) =>
          part.type === "text" ? (
            <div key={index} className="whitespace-pre-wrap break-words text-sm">
              {part.text}
            </div>
          ) : (
            <pre
              key={index}
              className="overflow-x-auto whitespace-pre-wrap break-words text-xs text-muted-foreground"
            >
              {JSON.stringify(part.data, null, 2)}
            </pre>
          ),
        )}
      </div>
    </div>
  );
}

/** Per-task drill-down: lifecycle timeline + the live message thread.
 *  Fetched lazily when the card is expanded; the list hook's SSE stream keeps
 *  the thread fresh by invalidating this task's detail key on task.message.*. */
function TaskDrilldown({ task, sessionId }: { task: SessionTask; sessionId: string | undefined }) {
  const { data: detail, isLoading, error } = useSessionTask(sessionId, task.id);
  const messages = detail?.messages ?? [];
  // Render the timeline from the freshly-fetched snapshot when available so the
  // lifecycle and the thread stay consistent; fall back to the list snapshot.
  const snapshot = detail?.task ?? task;

  return (
    <div className="mt-3 space-y-3 border-t border-border/60 pt-3">
      <div className="grid gap-0.5 text-xs text-muted-foreground">
        <div>Created {formatRelativeTime(snapshot.created_at)}</div>
        {snapshot.started_at ? <div>Started {formatRelativeTime(snapshot.started_at)}</div> : null}
        {snapshot.finished_at ? (
          <div>Finished {formatRelativeTime(snapshot.finished_at)}</div>
        ) : (
          <div>Last update {formatRelativeTime(snapshot.updated_at)}</div>
        )}
        {snapshot.state_detail ? <div>Detail: {snapshot.state_detail}</div> : null}
        {snapshot.attempt > 1 ? <div>Attempt: {snapshot.attempt}</div> : null}
      </div>

      <div className="space-y-2">
        <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <MessageSquare className="h-3.5 w-3.5" /> Message thread
        </div>
        {isLoading ? (
          <Skeleton className="h-16 w-full" />
        ) : error ? (
          <div className="text-xs text-destructive">
            {error instanceof Error ? error.message : "Failed to load thread"}
          </div>
        ) : messages.length === 0 ? (
          <div className="text-xs text-muted-foreground">
            No messages exchanged with this task yet.
          </div>
        ) : (
          <div className="space-y-1.5">
            {messages.map((message) => (
              <TaskMessageRow key={message.id} message={message} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** Full task detail card: header (name, kind, state, expand), body
 *  (created/id/status, progress, summary/error, result + artifact links, child
 *  session link, input prompt), and the expandable drill-down. */
export function TaskCard({
  task,
  sessionId,
}: {
  task: SessionTask;
  sessionId: string | undefined;
}) {
  const [expanded, setExpanded] = useState(false);
  const terminal = isTerminalTaskState(task.state);
  const artifacts = task.artifacts ?? [];
  const childSessionId = task.links?.child_session_id;

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <ListTodo className="h-4 w-4 text-muted-foreground" />
            <div>
              <CardTitle className="text-base">
                <EntityIdentity value={task.id}>{task.display_name || task.id}</EntityIdentity>
              </CardTitle>
              <CardDescription>
                <Badge variant="outline">{task.kind}</Badge>
              </CardDescription>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {taskStateBadge(task.state)}
            {!terminal && task.cancel_requested_at ? (
              <span className="text-xs text-muted-foreground">Cancel requested</span>
            ) : null}
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setExpanded((v) => !v)}
              aria-expanded={expanded}
              aria-label={expanded ? "Hide task details" : "Show task details"}
            >
              {expanded ? (
                <ChevronDown className="h-4 w-4" />
              ) : (
                <ChevronRight className="h-4 w-4" />
              )}
              Details
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-2 text-sm text-muted-foreground">
        <div className="grid gap-1">
          <div>Created: {formatRelativeTime(task.created_at)}</div>
          {task.state_detail ? <div>Status: {task.state_detail}</div> : null}
        </div>
        <TaskProgressBar task={task} />
        {terminal && task.summary ? (
          <div className="text-foreground">Summary: {task.summary}</div>
        ) : null}
        {task.error ? (
          <div className="text-destructive">
            {task.error.kind}: {task.error.message}
          </div>
        ) : null}
        {task.result_path ? (
          <div className="truncate">
            Result:{" "}
            {sessionId ? (
              // Link to the session Files tab; no query-param deep-link to a
              // specific path exists yet — add ?path= support when available.
              <Link
                href={`/sessions/${sessionId}/files`}
                className="text-primary underline-offset-4 hover:underline"
              >
                {task.result_path}
              </Link>
            ) : (
              task.result_path
            )}
          </div>
        ) : null}
        {artifacts.length > 0 ? (
          <div className="grid gap-1">
            {artifacts.map((artifact, index) => (
              <div key={`${artifact.name}-${index}`} className="truncate">
                {artifact.url ? (
                  <a
                    href={artifact.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-primary underline-offset-4 hover:underline"
                  >
                    {artifact.name}
                  </a>
                ) : artifact.type === "file" && artifact.path && sessionId ? (
                  // File artifacts with a VFS path link to the session Files
                  // tab. No query-param deep-link to a specific path exists
                  // yet — add ?path= support when available.
                  <Link
                    href={`/sessions/${sessionId}/files`}
                    className="text-primary underline-offset-4 hover:underline"
                  >
                    {artifact.name}
                    {` (${artifact.path})`}
                  </Link>
                ) : (
                  <span>
                    {artifact.name}
                    {artifact.path ? ` (${artifact.path})` : ""}
                  </span>
                )}
              </div>
            ))}
          </div>
        ) : null}
        {childSessionId ? (
          <div>
            <Link
              href={`/sessions/${childSessionId}/transcript`}
              className="text-primary underline-offset-4 hover:underline"
            >
              View child session
            </Link>
          </div>
        ) : null}
        <TaskInputPrompt task={task} />
        {expanded ? <TaskDrilldown task={task} sessionId={sessionId} /> : null}
      </CardContent>
    </Card>
  );
}
