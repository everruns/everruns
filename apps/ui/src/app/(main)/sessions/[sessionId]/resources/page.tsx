"use client";

import { AlertCircle, Bot, CheckCircle2, Clock3, ServerCrash, Wrench, XCircle } from "lucide-react";
import { useSessionResources } from "@/hooks/use-session-resources";
import { useSessionContext } from "../session-context";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { formatRelativeTime } from "@/lib/formatting";
import type { SessionResourceEntry } from "@/lib/api/types";

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
  const logPath = typeof resource.metadata?.log_path === "string" ? resource.metadata.log_path : null;
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

export default function ResourcesPage() {
  const { sessionId } = useSessionContext();
  const { data: resources, isLoading, error } = useSessionResources(sessionId);

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
          <span>{error instanceof Error ? error.message : "Failed to load resources"}</span>
        </div>
      </div>
    );
  }

  if (!resources || resources.length === 0) {
    return (
      <div className="flex-1 p-6">
        <div className="flex flex-col items-center justify-center h-64 text-muted-foreground">
          <Bot className="h-12 w-12 mb-4 opacity-50" />
          <p className="text-lg font-medium">No active resources</p>
          <p className="text-sm mt-1">
            Sandboxes, subagents, and other session resources will appear here.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 p-6 space-y-4">
      {resources.map((resource) => (
        <ResourceCard key={resource.resource_id} resource={resource} />
      ))}
    </div>
  );
}
