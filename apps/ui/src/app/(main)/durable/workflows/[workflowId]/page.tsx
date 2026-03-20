"use client";

import { use } from "react";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import {
  useWorkflow,
  useWorkflowEvents,
  useWorkflowSSE,
  useCancelWorkflow,
  useSignalWorkflow,
} from "@/hooks";
import type { WorkflowStatus, WorkflowEvent } from "@/lib/api/types";
import {
  AlertTriangle,
  CheckCircle,
  Clock,
  XCircle,
  Activity,
  RefreshCw,
  ArrowLeft,
  ExternalLink,
  Play,
  Pause,
  Timer,
  Zap,
  MessageSquare,
} from "lucide-react";
import { CopyButton } from "@/components/ui/copy-button";
import Link from "next/link";

function formatDistanceToNow(date: Date, options?: { addSuffix?: boolean }): string {
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  const seconds = Math.floor(diff / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  let result = "";
  if (days > 0) {
    result = `${days} day${days > 1 ? "s" : ""}`;
  } else if (hours > 0) {
    result = `${hours} hour${hours > 1 ? "s" : ""}`;
  } else if (minutes > 0) {
    result = `${minutes} minute${minutes > 1 ? "s" : ""}`;
  } else {
    result = "less than a minute";
  }

  return options?.addSuffix ? `${result} ago` : result;
}

function getStatusIcon(status: WorkflowStatus, size: "sm" | "lg" = "sm") {
  const sizeClass = size === "lg" ? "h-6 w-6" : "h-4 w-4";
  switch (status) {
    case "completed":
      return <CheckCircle className={`${sizeClass} text-green-500`} />;
    case "running":
      return <Activity className={`${sizeClass} text-blue-500 animate-pulse`} />;
    case "failed":
      return <XCircle className={`${sizeClass} text-red-500`} />;
    case "cancelled":
      return <AlertTriangle className={`${sizeClass} text-yellow-500`} />;
    default:
      return <Clock className={`${sizeClass} text-gray-500`} />;
  }
}

function getStatusBadgeVariant(status: WorkflowStatus) {
  switch (status) {
    case "completed":
      return "default" as const;
    case "running":
      return "secondary" as const;
    case "failed":
      return "destructive" as const;
    case "cancelled":
    case "pending":
      return "outline" as const;
    default:
      return "outline" as const;
  }
}

function formatEventType(eventType: string): string {
  // Convert snake_case to Title Case (e.g., "workflow_started" -> "Workflow Started")
  return eventType
    .split("_")
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

function getEventIcon(eventType: string) {
  // Event types from backend are snake_case (e.g., "workflow_started", "activity_completed")
  if (eventType.startsWith("workflow_")) {
    if (eventType === "workflow_started") return <Play className="h-4 w-4 text-green-500" />;
    if (eventType === "workflow_completed")
      return <CheckCircle className="h-4 w-4 text-green-500" />;
    if (eventType === "workflow_failed") return <XCircle className="h-4 w-4 text-red-500" />;
    if (eventType === "workflow_cancelled")
      return <AlertTriangle className="h-4 w-4 text-yellow-500" />;
    return <Activity className="h-4 w-4 text-blue-500" />;
  }
  if (eventType.startsWith("activity_")) {
    if (eventType === "activity_scheduled") return <Clock className="h-4 w-4 text-blue-500" />;
    if (eventType === "activity_started") return <Play className="h-4 w-4 text-blue-500" />;
    if (eventType === "activity_completed")
      return <CheckCircle className="h-4 w-4 text-green-500" />;
    if (eventType === "activity_failed") return <XCircle className="h-4 w-4 text-red-500" />;
    if (eventType === "activity_timed_out") return <Clock className="h-4 w-4 text-yellow-500" />;
    return <Activity className="h-4 w-4 text-gray-500" />;
  }
  if (eventType.startsWith("timer_")) {
    return <Timer className="h-4 w-4 text-purple-500" />;
  }
  if (eventType.startsWith("signal_")) {
    return <Zap className="h-4 w-4 text-yellow-500" />;
  }
  if (eventType.startsWith("child_workflow_")) {
    return <MessageSquare className="h-4 w-4 text-blue-500" />;
  }
  return <Activity className="h-4 w-4 text-gray-500" />;
}

function EventTimeline({ events }: { events: WorkflowEvent[] }) {
  if (events.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-8 text-muted-foreground">
        <Activity className="h-8 w-8 mb-2" />
        <p className="text-sm">No events yet</p>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {events.map((event, index) => (
        <div key={event.id} className="flex gap-4">
          <div className="flex flex-col items-center">
            <div className="flex h-8 w-8 items-center justify-center rounded-full bg-muted">
              {getEventIcon(event.event_type)}
            </div>
            {index < events.length - 1 && <div className="w-px h-full bg-border min-h-[20px]" />}
          </div>
          <div className="flex-1 pb-4">
            <div className="flex items-center justify-between">
              <p className="font-medium text-sm">{formatEventType(event.event_type)}</p>
              <span className="text-xs text-muted-foreground">
                #{event.sequence_num} ·{" "}
                {formatDistanceToNow(new Date(event.created_at), { addSuffix: true })}
              </span>
            </div>
            {event.event_data && Object.keys(event.event_data).length > 0 && (
              <div className="mt-2 p-2 rounded bg-muted/50">
                <pre className="text-xs overflow-auto max-h-32">
                  {JSON.stringify(event.event_data, null, 2)}
                </pre>
              </div>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

export default function WorkflowDetailPage({
  params,
}: {
  params: Promise<{ workflowId: string }>;
}) {
  const { workflowId } = use(params);

  // Connect to workflow-specific SSE for real-time updates
  useWorkflowSSE(workflowId);

  const {
    data: workflow,
    isLoading: workflowLoading,
    error: workflowError,
    refetch,
  } = useWorkflow(workflowId);
  const { data: events, isLoading: eventsLoading } = useWorkflowEvents(workflowId);
  const cancelMutation = useCancelWorkflow();
  const signalMutation = useSignalWorkflow();

  if (workflowLoading) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Workflow Details</h1>
        </div>
        <div className="space-y-6">
          <Skeleton className="h-8 w-32" />
          <div className="grid gap-6 md:grid-cols-2">
            <Skeleton className="h-64" />
            <Skeleton className="h-64" />
          </div>
          <Skeleton className="h-96" />
        </div>
      </div>
    );
  }

  if (workflowError || !workflow) {
    return (
      <div className="container mx-auto p-6">
        <div className="flex items-center justify-between mb-6">
          <h1 className="text-2xl font-bold">Workflow Details</h1>
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Workflow Not Found</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
              The workflow could not be loaded. It may not exist or the API is unavailable.
            </p>
            <div className="flex gap-2">
              <Link href="/durable/workflows">
                <Button variant="outline">
                  <ArrowLeft className="h-4 w-4 mr-2" />
                  Back to Workflows
                </Button>
              </Link>
              <Button onClick={() => refetch()} variant="outline">
                <RefreshCw className="h-4 w-4 mr-2" />
                Retry
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>
    );
  }

  const handleCancel = () => {
    if (confirm("Are you sure you want to cancel this workflow?")) {
      cancelMutation.mutate(workflowId);
    }
  };

  const handleSignal = (signalType: string) => {
    if (confirm(`Send "${signalType}" signal to this workflow?`)) {
      signalMutation.mutate({ workflowId, signalType });
    }
  };

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <h1 className="text-2xl font-bold">Workflow Details</h1>
      </div>
      <div className="space-y-6">
        {/* Back link */}
        <Link
          href="/durable/workflows"
          className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4 mr-1" />
          Back to Workflows
        </Link>

        {/* Header */}
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-4">
            {getStatusIcon(workflow.status, "lg")}
            <div className="flex items-center gap-2">
              <h2 className="text-2xl font-bold">{workflow.workflow_type}</h2>
              <CopyButton value={workflow.id} />
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Badge variant={getStatusBadgeVariant(workflow.status)} className="text-lg px-3 py-1">
              {workflow.status}
            </Badge>
            {workflow.status === "running" && (
              <>
                <Button variant="outline" onClick={() => handleSignal("shutdown")}>
                  <Pause className="h-4 w-4 mr-2" />
                  Shutdown
                </Button>
                <Button variant="destructive" onClick={handleCancel}>
                  Cancel
                </Button>
              </>
            )}
          </div>
        </div>

        {/* Info cards */}
        <div className="grid gap-6 md:grid-cols-2">
          <Card>
            <CardHeader>
              <CardTitle>Details</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <p className="text-sm text-muted-foreground">Created</p>
                  <p className="font-medium">{new Date(workflow.created_at).toLocaleString()}</p>
                </div>
                {workflow.started_at && (
                  <div>
                    <p className="text-sm text-muted-foreground">Started</p>
                    <p className="font-medium">{new Date(workflow.started_at).toLocaleString()}</p>
                  </div>
                )}
                {workflow.completed_at && (
                  <div>
                    <p className="text-sm text-muted-foreground">Completed</p>
                    <p className="font-medium">
                      {new Date(workflow.completed_at).toLocaleString()}
                    </p>
                  </div>
                )}
              </div>

              {workflow.session_id && (
                <>
                  <Separator />
                  <div>
                    <p className="text-sm text-muted-foreground mb-1">Linked Session</p>
                    <Link
                      href={`/sessions/${workflow.session_id}`}
                      className="inline-flex items-center gap-1 text-primary hover:underline"
                    >
                      <ExternalLink className="h-4 w-4" />
                      View Session
                    </Link>
                  </div>
                </>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Input</CardTitle>
              <CardDescription>Workflow input parameters</CardDescription>
            </CardHeader>
            <CardContent>
              <ScrollArea className="h-[150px]">
                <pre className="text-sm bg-muted p-3 rounded">
                  {JSON.stringify(workflow.input, null, 2)}
                </pre>
              </ScrollArea>
            </CardContent>
          </Card>
        </div>

        {/* Result/Error */}
        {(workflow.result || workflow.error) && (
          <Card>
            <CardHeader>
              <CardTitle>{workflow.error ? "Error" : "Result"}</CardTitle>
            </CardHeader>
            <CardContent>
              <ScrollArea className="h-[200px]">
                <pre
                  className={`text-sm p-3 rounded ${workflow.error ? "bg-red-500/10" : "bg-muted"}`}
                >
                  {JSON.stringify(workflow.error || workflow.result, null, 2)}
                </pre>
              </ScrollArea>
            </CardContent>
          </Card>
        )}

        {/* Event Timeline */}
        <Card>
          <CardHeader className="flex flex-row items-center justify-between">
            <div>
              <CardTitle>Event History</CardTitle>
              <CardDescription>Timeline of workflow events</CardDescription>
            </div>
            <Button variant="outline" size="sm" onClick={() => refetch()}>
              <RefreshCw className="h-4 w-4 mr-2" />
              Refresh
            </Button>
          </CardHeader>
          <CardContent>
            {eventsLoading ? (
              <div className="space-y-4">
                {[...Array(3)].map((_, i) => (
                  <Skeleton key={i} className="h-20" />
                ))}
              </div>
            ) : (
              <ScrollArea className="h-[400px] pr-4">
                <EventTimeline events={events || []} />
              </ScrollArea>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
