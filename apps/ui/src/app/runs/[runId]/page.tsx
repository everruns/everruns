"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { useRun, useCancelRun } from "@/hooks/use-runs";
import { useAgent } from "@/hooks/use-agents";
import { useSSEEvents, aggregateTextMessages, aggregateToolCalls } from "@/hooks/use-sse-events";
import { Header } from "@/components/layout/header";
import { RunStatusBadge } from "@/components/runs/run-status-badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Separator } from "@/components/ui/separator";
import { ArrowLeft, Play, Square, Bot, Wrench, MessageSquare, Wifi, WifiOff, Loader2 } from "lucide-react";
import type { RunStatus, AgUiEvent } from "@/lib/api/types";

function EventItem({ event }: { event: AgUiEvent }) {
  const getEventIcon = () => {
    switch (event.type) {
      case "RUN_STARTED":
      case "RUN_FINISHED":
        return <Play className="h-4 w-4" />;
      case "RUN_ERROR":
        return <Square className="h-4 w-4 text-destructive" />;
      case "TEXT_MESSAGE_START":
      case "TEXT_MESSAGE_CHUNK":
      case "TEXT_MESSAGE_END":
        return <MessageSquare className="h-4 w-4" />;
      case "TOOL_CALL_START":
      case "TOOL_CALL_RESULT":
        return <Wrench className="h-4 w-4" />;
      default:
        return <Bot className="h-4 w-4" />;
    }
  };

  const getEventColor = () => {
    switch (event.type) {
      case "RUN_STARTED":
        return "bg-blue-100 text-blue-800";
      case "RUN_FINISHED":
        return "bg-green-100 text-green-800";
      case "RUN_ERROR":
        return "bg-red-100 text-red-800";
      case "TOOL_CALL_START":
      case "TOOL_CALL_RESULT":
        return "bg-purple-100 text-purple-800";
      default:
        return "bg-gray-100 text-gray-800";
    }
  };

  const renderEventDetails = () => {
    switch (event.type) {
      case "TEXT_MESSAGE_CHUNK":
        return (
          <span className="font-mono text-sm">{event.chunk}</span>
        );
      case "TEXT_MESSAGE_START":
        return (
          <span className="text-muted-foreground">
            Message started (role: {event.role})
          </span>
        );
      case "TOOL_CALL_START":
        return (
          <div className="space-y-1">
            <span className="font-medium">{event.tool_name}</span>
            <pre className="text-xs bg-muted p-2 rounded overflow-x-auto">
              {JSON.stringify(event.arguments, null, 2)}
            </pre>
          </div>
        );
      case "TOOL_CALL_RESULT":
        return (
          <div className="space-y-1">
            {event.error ? (
              <span className="text-destructive">{event.error}</span>
            ) : (
              <pre className="text-xs bg-muted p-2 rounded overflow-x-auto max-h-32">
                {JSON.stringify(event.result, null, 2)}
              </pre>
            )}
          </div>
        );
      case "RUN_ERROR":
        return <span className="text-destructive">{event.error}</span>;
      default:
        return null;
    }
  };

  return (
    <div className="flex gap-3 py-2">
      <div className={`p-1.5 rounded ${getEventColor()}`}>
        {getEventIcon()}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-1">
          <Badge variant="outline" className="text-xs">
            {event.type}
          </Badge>
          <span className="text-xs text-muted-foreground">
            {new Date(event.timestamp).toLocaleTimeString()}
          </span>
        </div>
        {renderEventDetails()}
      </div>
    </div>
  );
}

function RunEventStream({ runId, status }: { runId: string; status: RunStatus }) {
  const isTerminal = status === "completed" || status === "failed" || status === "cancelled";

  const { events, isConnected, error } = useSSEEvents({
    runId,
    enabled: !isTerminal,
  });

  const aggregatedMessages = aggregateTextMessages(events);
  const aggregatedToolCalls = aggregateToolCalls(events);

  return (
    <div className="space-y-4">
      {/* Connection Status */}
      <div className="flex items-center gap-2 text-sm">
        {isTerminal ? (
          <>
            <WifiOff className="h-4 w-4 text-muted-foreground" />
            <span className="text-muted-foreground">Run completed - showing cached events</span>
          </>
        ) : isConnected ? (
          <>
            <Wifi className="h-4 w-4 text-green-600" />
            <span className="text-green-600">Connected - streaming events</span>
          </>
        ) : error ? (
          <>
            <WifiOff className="h-4 w-4 text-destructive" />
            <span className="text-destructive">Connection error</span>
          </>
        ) : (
          <>
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>Connecting...</span>
          </>
        )}
      </div>

      {/* Aggregated Content */}
      {aggregatedMessages.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">Messages</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-4">
              {aggregatedMessages.map((msg) => (
                <div key={msg.id} className="space-y-1">
                  <Badge variant="outline">{msg.role}</Badge>
                  <p className="whitespace-pre-wrap">{msg.content}</p>
                  {!msg.isComplete && (
                    <span className="inline-block w-2 h-4 bg-primary animate-pulse" />
                  )}
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {aggregatedToolCalls.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">Tool Calls</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-3">
              {aggregatedToolCalls.map((tc) => (
                <div key={tc.id} className="border rounded-lg p-3">
                  <div className="flex items-center gap-2 mb-2">
                    <Wrench className="h-4 w-4" />
                    <span className="font-medium">{tc.name}</span>
                    {tc.isComplete ? (
                      tc.error ? (
                        <Badge variant="destructive">Failed</Badge>
                      ) : (
                        <Badge variant="outline" className="bg-green-100 text-green-800">
                          Success
                        </Badge>
                      )
                    ) : (
                      <Badge variant="outline" className="animate-pulse">
                        Running...
                      </Badge>
                    )}
                  </div>
                  <pre className="text-xs bg-muted p-2 rounded overflow-x-auto max-h-24">
                    {JSON.stringify(tc.arguments, null, 2)}
                  </pre>
                  {tc.isComplete && tc.result && (
                    <>
                      <Separator className="my-2" />
                      <pre className="text-xs bg-muted p-2 rounded overflow-x-auto max-h-24">
                        {JSON.stringify(tc.result, null, 2)}
                      </pre>
                    </>
                  )}
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Raw Events */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">Event Log</CardTitle>
          <CardDescription>{events.length} events</CardDescription>
        </CardHeader>
        <CardContent>
          <ScrollArea className="h-96">
            {events.length === 0 ? (
              <p className="text-muted-foreground text-center py-8">
                Waiting for events...
              </p>
            ) : (
              <div className="divide-y">
                {events.map((event, index) => (
                  <EventItem key={index} event={event} />
                ))}
              </div>
            )}
          </ScrollArea>
        </CardContent>
      </Card>
    </div>
  );
}

export default function RunDetailPage() {
  const params = useParams();
  const runId = params.runId as string;

  const { data: run, isLoading: runLoading, error: runError } = useRun(runId);
  const { data: agent } = useAgent(run?.agent_id || "");
  const cancelRun = useCancelRun();

  const handleCancel = async () => {
    if (run && (run.status === "pending" || run.status === "running")) {
      await cancelRun.mutateAsync(runId);
    }
  };

  if (runLoading) {
    return (
      <>
        <Header title="Run Details" />
        <div className="p-6 space-y-6">
          <Skeleton className="h-48" />
          <Skeleton className="h-96" />
        </div>
      </>
    );
  }

  if (runError || !run) {
    return (
      <>
        <Header
          title="Run Not Found"
          action={
            <Link href="/runs">
              <Button variant="ghost">
                <ArrowLeft className="h-4 w-4 mr-2" />
                Back to Runs
              </Button>
            </Link>
          }
        />
        <div className="p-6">
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg">
            {runError?.message || "Run not found"}
          </div>
        </div>
      </>
    );
  }

  const canCancel = run.status === "pending" || run.status === "running";

  return (
    <>
      <Header
        title={`Run ${run.id.slice(0, 8)}...`}
        action={
          <div className="flex gap-2">
            <Link href="/runs">
              <Button variant="ghost">
                <ArrowLeft className="h-4 w-4 mr-2" />
                Back
              </Button>
            </Link>
            {canCancel && (
              <Button
                variant="destructive"
                onClick={handleCancel}
                disabled={cancelRun.isPending}
              >
                {cancelRun.isPending ? (
                  <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                ) : (
                  <Square className="h-4 w-4 mr-2" />
                )}
                Cancel Run
              </Button>
            )}
          </div>
        }
      />
      <div className="p-6 space-y-6">
        {/* Run Info Card */}
        <Card>
          <CardHeader className="flex flex-row items-start justify-between">
            <div>
              <CardTitle className="font-mono text-lg">{run.id}</CardTitle>
              <CardDescription>
                Agent: {agent?.name || run.agent_id} v{run.agent_version}
              </CardDescription>
            </div>
            <RunStatusBadge status={run.status as RunStatus} size="lg" />
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
              <div>
                <p className="text-muted-foreground">Created</p>
                <p className="font-medium">
                  {new Date(run.created_at).toLocaleString()}
                </p>
              </div>
              <div>
                <p className="text-muted-foreground">Started</p>
                <p className="font-medium">
                  {run.started_at
                    ? new Date(run.started_at).toLocaleString()
                    : "-"}
                </p>
              </div>
              <div>
                <p className="text-muted-foreground">Finished</p>
                <p className="font-medium">
                  {run.finished_at
                    ? new Date(run.finished_at).toLocaleString()
                    : "-"}
                </p>
              </div>
              <div>
                <p className="text-muted-foreground">Thread</p>
                <Link
                  href={`/threads/${run.thread_id}`}
                  className="font-medium font-mono text-primary hover:underline"
                >
                  {run.thread_id.slice(0, 8)}...
                </Link>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* Event Stream */}
        <RunEventStream runId={runId} status={run.status as RunStatus} />
      </div>
    </>
  );
}
