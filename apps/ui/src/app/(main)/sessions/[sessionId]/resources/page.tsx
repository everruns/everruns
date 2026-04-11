"use client";

import {
  AlertCircle,
  Bot,
  CheckCircle2,
  Clock3,
  Loader2,
  RefreshCcw,
  ServerCrash,
  Wrench,
  XCircle,
} from "lucide-react";
import { useSessionResources } from "@/hooks/use-session-resources";
import { useSessionContext } from "../session-context";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { formatRelativeTime } from "@/lib/formatting";
import type {
  LeasedSessionResource,
  SessionResource,
  SubagentSessionResource,
} from "@/lib/api/types";

// ---------------------------------------------------------------------------
// Leased resource card
// ---------------------------------------------------------------------------

function leasedStatusBadge(resource: LeasedSessionResource) {
  switch (resource.status) {
    case "active":
      return <Badge variant="default">Active</Badge>;
    case "cleaning":
      return <Badge variant="secondary">Cleaning</Badge>;
    case "released":
      return <Badge variant="outline">Released</Badge>;
    case "cleanup_failed":
      return <Badge variant="destructive">Cleanup Failed</Badge>;
  }
}

function leasedStatusIcon(resource: LeasedSessionResource) {
  switch (resource.status) {
    case "active":
      return <Clock3 className="h-4 w-4 text-muted-foreground" />;
    case "cleaning":
      return <RefreshCcw className="h-4 w-4 text-muted-foreground" />;
    case "released":
      return <CheckCircle2 className="h-4 w-4 text-muted-foreground" />;
    case "cleanup_failed":
      return <ServerCrash className="h-4 w-4 text-destructive" />;
  }
}

function LeasedResourceCard({ resource }: { resource: LeasedSessionResource }) {
  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            {leasedStatusIcon(resource)}
            <div>
              <CardTitle className="text-base">
                {resource.display_name ?? resource.external_id}
              </CardTitle>
              <CardDescription>
                {resource.provider} / {resource.resource_type}
              </CardDescription>
            </div>
          </div>
          {leasedStatusBadge(resource)}
        </div>
      </CardHeader>
      <CardContent className="space-y-2 text-sm text-muted-foreground">
        <div className="grid gap-1">
          <div>Last touched: {formatRelativeTime(resource.last_touched_at)}</div>
          <div>Lease expires: {formatRelativeTime(resource.lease_expires_at)}</div>
          <div>Cleanup attempts: {resource.cleanup_attempts}</div>
          <div className="truncate">External ID: {resource.external_id}</div>
        </div>
        {resource.last_cleanup_error && (
          <div className="rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-destructive">
            {resource.last_cleanup_error}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Subagent resource card
// ---------------------------------------------------------------------------

function subagentStatusBadge(resource: SubagentSessionResource) {
  switch (resource.status) {
    case "spawning":
      return <Badge variant="secondary">Spawning</Badge>;
    case "running":
      return <Badge variant="default">Running</Badge>;
    case "completed":
      return <Badge variant="outline">Completed</Badge>;
    case "failed":
      return <Badge variant="destructive">Failed</Badge>;
    case "cancelled":
      return <Badge variant="secondary">Cancelled</Badge>;
    case "max_iterations_reached":
      return <Badge variant="secondary">Max Iterations</Badge>;
  }
}

function subagentStatusIcon(resource: SubagentSessionResource) {
  switch (resource.status) {
    case "spawning":
    case "running":
      return <Loader2 className="h-4 w-4 text-muted-foreground animate-spin" />;
    case "completed":
      return <CheckCircle2 className="h-4 w-4 text-muted-foreground" />;
    case "failed":
      return <XCircle className="h-4 w-4 text-destructive" />;
    case "cancelled":
    case "max_iterations_reached":
      return <AlertCircle className="h-4 w-4 text-muted-foreground" />;
  }
}

function SubagentResourceCard({ resource }: { resource: SubagentSessionResource }) {
  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            {subagentStatusIcon(resource)}
            <div>
              <CardTitle className="text-base">{resource.name}</CardTitle>
              <CardDescription>subagent</CardDescription>
            </div>
          </div>
          {subagentStatusBadge(resource)}
        </div>
      </CardHeader>
      <CardContent className="space-y-2 text-sm text-muted-foreground">
        {resource.task && <div className="line-clamp-2">{resource.task}</div>}
        <div className="grid gap-1">
          <div>Started: {formatRelativeTime(resource.created_at)}</div>
          {resource.blueprint_id && (
            <div className="truncate">Blueprint: {resource.blueprint_id}</div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

// ---------------------------------------------------------------------------
// Unified resource card
// ---------------------------------------------------------------------------

function ResourceCard({ resource }: { resource: SessionResource }) {
  switch (resource.kind) {
    case "leased":
      return <LeasedResourceCard resource={resource} />;
    case "subagent":
      return <SubagentResourceCard resource={resource} />;
  }
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

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
            Sandboxes, browser sessions, and subagents will appear here.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 p-6 space-y-4">
      {resources.map((resource) => (
        <ResourceCard
          key={resource.kind === "leased" ? resource.id : resource.session_id}
          resource={resource}
        />
      ))}
    </div>
  );
}
