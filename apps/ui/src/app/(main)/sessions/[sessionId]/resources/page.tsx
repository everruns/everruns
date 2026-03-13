"use client";

import { AlertCircle, CheckCircle2, Clock3, RefreshCcw, ServerCrash, Wrench } from "lucide-react";
import { useSessionResources } from "@/hooks/use-session-resources";
import { useSessionContext } from "../session-context";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { formatRelativeTime } from "@/lib/formatting";
import type { LeasedResource } from "@/lib/api/types";

function statusBadge(resource: LeasedResource) {
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

function statusIcon(resource: LeasedResource) {
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

function ResourceCard({ resource }: { resource: LeasedResource }) {
  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            {statusIcon(resource)}
            <div>
              <CardTitle className="text-base">
                {resource.display_name ?? resource.external_id}
              </CardTitle>
              <CardDescription>
                {resource.provider} / {resource.resource_type}
              </CardDescription>
            </div>
          </div>
          {statusBadge(resource)}
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
          <Wrench className="h-12 w-12 mb-4 opacity-50" />
          <p className="text-lg font-medium">No leased resources</p>
          <p className="text-sm mt-1">
            Provider-managed resources that require cleanup will appear here.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 p-6 space-y-4">
      {resources.map((resource) => (
        <ResourceCard key={resource.id} resource={resource} />
      ))}
    </div>
  );
}
