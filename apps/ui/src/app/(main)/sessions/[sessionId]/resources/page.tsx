"use client";

// Tasks + Resources page. Session tasks (specs/session-tasks.md) render on
// top with live SSE-driven updates; the leased-resource registry stays below
// unchanged. The nav tab keeps the "Resources" label and leased_resources
// feature gating.

import { useState } from "react";
import Link from "next/link";
import {
  AlertCircle,
  Bot,
  CheckCircle2,
  Clock3,
  ListTodo,
  ServerCrash,
  Wrench,
  XCircle,
} from "lucide-react";
import { useSessionResources } from "@/hooks/use-session-resources";
import {
  useCancelSessionTask,
  useSendTaskMessage,
  useSessionTasks,
} from "@/hooks/use-session-tasks";
import { useSessionContext } from "../session-context";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { formatRelativeTime } from "@/lib/formatting";
import { isTerminalTaskState } from "@/lib/api/types";
import type { SessionResourceEntry, SessionTask, SessionTaskState } from "@/lib/api/types";

// ============================================
// Tasks
// ============================================

function taskStateBadge(state: SessionTaskState) {
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

function TaskProgressBar({ task }: { task: SessionTask }) {
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

function TaskInputPrompt({
  task,
  sessionId,
}: {
  task: SessionTask;
  sessionId: string | undefined;
}) {
  const [text, setText] = useState("");
  const sendMessage = useSendTaskMessage(sessionId);

  if (task.state !== "awaiting_input" || !task.input_request) return null;
  const inputRequest = task.input_request;

  const send = () => {
    const trimmed = text.trim();
    if (!trimmed || sendMessage.isPending) return;
    sendMessage.mutate(
      {
        taskId: task.id,
        request: {
          content: [{ type: "text", text: trimmed }],
          in_reply_to: inputRequest.id,
        },
      },
      { onSuccess: () => setText("") },
    );
  };

  return (
    <div className="space-y-2 rounded border border-amber-500/50 bg-amber-500/10 px-3 py-2">
      <div className="text-sm text-foreground">{inputRequest.prompt}</div>
      <div className="flex items-center gap-2">
        <Input
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") send();
          }}
          placeholder="Type a response..."
          aria-label="Task input response"
        />
        <Button size="sm" onClick={send} disabled={!text.trim() || sendMessage.isPending}>
          Send
        </Button>
      </div>
      {sendMessage.isError ? (
        <div className="text-xs text-destructive">
          {sendMessage.error instanceof Error ? sendMessage.error.message : "Failed to send"}
        </div>
      ) : null}
    </div>
  );
}

function TaskCard({ task, sessionId }: { task: SessionTask; sessionId: string | undefined }) {
  const cancelTask = useCancelSessionTask(sessionId);
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
              <CardTitle className="text-base">{task.display_name || task.id}</CardTitle>
              <CardDescription>
                <Badge variant="outline">{task.kind}</Badge>
              </CardDescription>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {taskStateBadge(task.state)}
            {terminal ? null : task.cancel_requested_at ? (
              <span className="text-xs text-muted-foreground">Cancel requested</span>
            ) : (
              <Button
                variant="outline"
                size="sm"
                onClick={() => cancelTask.mutate(task.id)}
                disabled={cancelTask.isPending}
              >
                Cancel
              </Button>
            )}
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-2 text-sm text-muted-foreground">
        <div className="grid gap-1">
          <div>Created: {formatRelativeTime(task.created_at)}</div>
          <div className="truncate">ID: {task.id}</div>
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
        {task.result_path ? <div className="truncate">Result: {task.result_path}</div> : null}
        {artifacts.length > 0 ? (
          <div className="grid gap-1">
            {artifacts.map((artifact, index) => (
              <div key={`${artifact.name}-${index}`} className="truncate">
                {/* No VFS deep-link route exists; paths render as plain text. */}
                {artifact.url ? (
                  <a
                    href={artifact.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-primary underline-offset-4 hover:underline"
                  >
                    {artifact.name}
                  </a>
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
              href={`/sessions/${childSessionId}`}
              className="text-primary underline-offset-4 hover:underline"
            >
              View child session
            </Link>
          </div>
        ) : null}
        <TaskInputPrompt task={task} sessionId={sessionId} />
      </CardContent>
    </Card>
  );
}

function TasksSection({ sessionId }: { sessionId: string | undefined }) {
  const { data: tasks, isLoading, error } = useSessionTasks(sessionId);

  return (
    <section className="space-y-3">
      <h2 className="text-sm font-medium text-muted-foreground">Tasks</h2>
      {isLoading ? (
        <Skeleton className="h-32 w-full" />
      ) : error ? (
        <div className="flex items-center gap-2 text-destructive">
          <AlertCircle className="h-5 w-5" />
          <span>{error instanceof Error ? error.message : "Failed to load tasks"}</span>
        </div>
      ) : !tasks || tasks.length === 0 ? (
        <div className="text-sm text-muted-foreground">
          No tasks yet. Subagents, external agents, and background work will appear here.
        </div>
      ) : (
        tasks.map((task) => <TaskCard key={task.id} task={task} sessionId={sessionId} />)
      )}
    </section>
  );
}

// ============================================
// Resources (unchanged)
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
                {resource.display_name || resource.resource_id}
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
          <div className="truncate">ID: {resource.resource_id}</div>
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

function ResourcesSection({ sessionId }: { sessionId: string | undefined }) {
  const { data: resources, isLoading, error } = useSessionResources(sessionId);

  return (
    <section className="space-y-3">
      <h2 className="text-sm font-medium text-muted-foreground">Resources</h2>
      {isLoading ? (
        <Skeleton className="h-32 w-full" />
      ) : error ? (
        <div className="flex items-center gap-2 text-destructive">
          <AlertCircle className="h-5 w-5" />
          <span>{error instanceof Error ? error.message : "Failed to load resources"}</span>
        </div>
      ) : !resources || resources.length === 0 ? (
        <div className="flex flex-col items-center justify-center h-40 text-muted-foreground">
          <Bot className="h-12 w-12 mb-4 opacity-50" />
          <p className="text-lg font-medium">No active resources</p>
          <p className="text-sm mt-1">
            Sandboxes, subagents, and other session resources will appear here.
          </p>
        </div>
      ) : (
        resources.map((resource) => <ResourceCard key={resource.resource_id} resource={resource} />)
      )}
    </section>
  );
}

export default function ResourcesPage() {
  const { sessionId } = useSessionContext();

  return (
    <div className="flex-1 p-6 space-y-8">
      <TasksSection sessionId={sessionId} />
      <ResourcesSection sessionId={sessionId} />
    </div>
  );
}
