"use client";
import { useConfirmation } from "@/components/ui/confirmation-provider";
import { PageContainer, PageMasthead } from "@/components/layout";

import { useUiTranslations, useLocale } from "@/providers/locale-provider";
import { useDurablePageRows } from "@/components/chat-mode/use-durable-page-rows";

import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { useWorkers, useDrainWorker, useResumeWorker, usePageTitle } from "@/hooks";
import type { DurableWorker, WorkerStatus } from "@/lib/api/types";
import {
  Server,
  AlertTriangle,
  Activity,
  Clock,
  RefreshCw,
  Pause,
  Play,
  CheckCircle,
} from "lucide-react";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { formatDistanceToNow, formatDurationCompact } from "@/lib/formatting";
import { getWorkerStatusBadgeVariant } from "@/lib/status-utils";

function getStatusColor(status: WorkerStatus) {
  switch (status) {
    case "active":
      return "bg-success";
    case "draining":
      return "bg-warning";
    case "stopped":
      return "bg-destructive";
    case "stale":
      return "bg-muted-foreground";
    default:
      return "bg-muted-foreground";
  }
}

function WorkerRow({
  worker,
  onDrain,
  onResume,
}: {
  worker: DurableWorker;
  onDrain: (id: string) => void;
  onResume: (id: string) => void;
}) {
  const { displayLocale } = useLocale();

  const ui = useUiTranslations();
  const loadPercentage =
    worker.max_concurrency > 0 ? (worker.current_load / worker.max_concurrency) * 100 : 0;

  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-2">
          <div className={`w-2 h-2 rounded-full ${getStatusColor(worker.status)}`} />
          <div className="font-medium">
            <EntityIdentity value={worker.id}>
              {worker.hostname || worker.id.slice(0, 20)}
            </EntityIdentity>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant={getWorkerStatusBadgeVariant(worker.status)}>{worker.status}</Badge>
      </TableCell>
      <TableCell>
        <Badge variant="outline">{worker.worker_group}</Badge>
      </TableCell>
      <TableCell>
        <div className="space-y-1">
          <div className="flex items-center justify-between text-sm">
            <span>
              {worker.current_load}/{worker.max_concurrency}
            </span>
            <span className="text-muted-foreground text-xs">{loadPercentage.toFixed(0)}%</span>
          </div>
          <div className="h-1.5 bg-muted rounded-full overflow-hidden w-20">
            <div
              className={`h-full transition-all ${
                loadPercentage > 80
                  ? "bg-red-500"
                  : loadPercentage > 60
                    ? "bg-yellow-500"
                    : "bg-green-500"
              }`}
              style={{ width: `${loadPercentage}%` }}
            />
          </div>
        </div>
      </TableCell>
      <TableCell>
        {worker.accepting_tasks ? (
          <div className="flex items-center gap-1 text-success">
            <CheckCircle className="h-3 w-3" />
            <span className="text-xs">{ui("Yes")}</span>
          </div>
        ) : (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger>
                <div className="flex items-center gap-1 text-warning">
                  <Pause className="h-3 w-3" />
                  <span className="text-xs">{ui("No")}</span>
                </div>
              </TooltipTrigger>
              <TooltipContent>
                {worker.backpressure_reason || ui("Under backpressure")}
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        )}
      </TableCell>
      <TableCell>
        <div className="flex flex-wrap gap-1">
          {worker.activity_types.slice(0, 3).map((type) => (
            <Badge key={type} variant="outline" className="text-xs">
              {type}
            </Badge>
          ))}
          {worker.activity_types.length > 3 && (
            <Badge variant="outline" className="text-xs">
              +{worker.activity_types.length - 3}
            </Badge>
          )}
        </div>
      </TableCell>
      <TableCell>
        <div className="text-sm">
          <p className="text-success">
            {(worker.tasks_completed ?? 0).toLocaleString(displayLocale)}
          </p>
          {(worker.tasks_failed ?? 0) > 0 && (
            <p className="text-xs text-destructive">
              {worker.tasks_failed} {ui("failed")}
            </p>
          )}
        </div>
      </TableCell>
      <TableCell>
        <span className="text-sm">{formatDurationCompact(worker.avg_task_duration_ms ?? 0)}</span>
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-muted-foreground">
              {formatDistanceToNow(
                new Date(worker.last_heartbeat_at),
                { addSuffix: true },
                displayLocale,
              )}
            </TooltipTrigger>
            <TooltipContent>
              {new Date(worker.last_heartbeat_at).toLocaleString(displayLocale)}
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        {worker.status === "active" && (
          <Button variant="outline" size="sm" onClick={() => onDrain(worker.id)}>
            <Pause className="h-3 w-3 mr-1" />
            {ui("Drain")}
          </Button>
        )}
        {worker.status === "draining" && (
          <Button variant="outline" size="sm" onClick={() => onResume(worker.id)}>
            <Play className="h-3 w-3 mr-1" />
            {ui("Resume")}
          </Button>
        )}
      </TableCell>
    </TableRow>
  );
}

export default function WorkersPage() {
  const requestConfirmation = useConfirmation();
  const ui = useUiTranslations();
  usePageTitle(ui("Workers"), ui("Durable"));
  const { data, isLoading, error, refetch } = useWorkers();
  const drainMutation = useDrainWorker();
  const resumeMutation = useResumeWorker();

  useDurablePageRows(
    "Workers",
    (data?.data ?? []).map((w) => ({
      id: w.id,
      status: w.status,
      group: w.worker_group,
      current_load: w.current_load,
      max_concurrency: w.max_concurrency,
      heartbeat_at: w.last_heartbeat_at,
    })),
    { loading: isLoading, error: !!error },
  );
  if (isLoading) {
    return (
      <PageContainer className="operational-page">
        <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
          <PageMasthead title={ui("Workers")} className="min-w-0 flex-1" />
        </div>
        <div className="space-y-6">
          <div className="grid gap-4 md:grid-cols-4">
            {[...Array(4)].map((_, i) => (
              <Skeleton key={i} className="h-24" />
            ))}
          </div>
          <Skeleton className="h-96" />
        </div>
      </PageContainer>
    );
  }

  if (error) {
    return (
      <PageContainer className="operational-page">
        <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
          <PageMasthead title={ui("Workers")} className="min-w-0 flex-1" />
        </div>
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12">
            <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">{ui("Unable to Load Workers")}</h3>
            <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
              {ui(
                "The durable workers API is not available. Please ensure the backend is running.",
              )}
            </p>
            <Button onClick={() => refetch()} variant="outline">
              <RefreshCw className="h-4 w-4 mr-2" />
              {ui("Retry")}
            </Button>
          </CardContent>
        </Card>
      </PageContainer>
    );
  }

  const summary = data?.summary;
  const workers = data?.data || [];

  const summaryStats = [
    {
      title: "Total Workers",
      value: data?.total || 0,
      description: `${summary?.active || 0} active`,
      icon: Server,
      color: "text-blue-600",
    },
    {
      title: "Total Capacity",
      value: summary?.total_capacity || 0,
      description: `${summary?.total_load || 0} in use`,
      icon: Activity,
      color: "text-green-600",
    },
    {
      title: "Load",
      value: summary?.total_capacity
        ? `${((summary.total_load / summary.total_capacity) * 100).toFixed(1)}%`
        : "0%",
      description: `${summary?.total_load || 0}/${summary?.total_capacity || 0} slots`,
      icon: Clock,
      color: "text-yellow-600",
    },
    {
      title: "Draining",
      value: summary?.draining || 0,
      description: `${summary?.stopped || 0} stopped`,
      icon: Pause,
      color: "text-orange-600",
    },
  ];

  const handleDrain = async (workerId: string) => {
    if (
      await requestConfirmation(
        `Are you sure you want to drain worker ${workerId.slice(0, 12)}...?`,
      )
    ) {
      drainMutation.mutate(workerId);
    }
  };

  const handleResume = (workerId: string) => {
    resumeMutation.mutate(workerId);
  };

  return (
    <PageContainer className="operational-page">
      <div className="operational-toolbar flex flex-wrap items-center justify-between gap-3 mb-6">
        <PageMasthead title={ui("Workers")} className="min-w-0 flex-1" />
      </div>
      <div className="space-y-6">
        {/* Summary Stats */}
        <div className="operational-stats grid gap-4 md:grid-cols-2 lg:grid-cols-4">
          {summaryStats.map((stat) => (
            <Card key={stat.title}>
              <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
                <CardTitle className="text-sm font-medium">{stat.title}</CardTitle>
                <stat.icon className={`h-4 w-4 ${stat.color}`} />
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{stat.value}</div>
                <p className="text-xs text-muted-foreground">{stat.description}</p>
              </CardContent>
            </Card>
          ))}
        </div>

        {/* Workers Table */}
        <Card>
          <CardHeader>
            <CardTitle>{ui("Worker Pool")}</CardTitle>
            <CardDescription>
              {ui("All registered workers and their current status")}
            </CardDescription>
          </CardHeader>
          <CardContent>
            {workers.length > 0 ? (
              <div className="border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>{ui("Worker")}</TableHead>
                      <TableHead>{ui("Status")}</TableHead>
                      <TableHead>{ui("Group")}</TableHead>
                      <TableHead>{ui("Load")}</TableHead>
                      <TableHead>{ui("Accepting")}</TableHead>
                      <TableHead>{ui("Activities")}</TableHead>
                      <TableHead>{ui("Completed")}</TableHead>
                      <TableHead>{ui("Avg Duration")}</TableHead>
                      <TableHead>{ui("Last Heartbeat")}</TableHead>
                      <TableHead>{ui("Actions")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {workers.map((worker) => (
                      <WorkerRow
                        key={worker.id}
                        worker={worker}
                        onDrain={handleDrain}
                        onResume={handleResume}
                      />
                    ))}
                  </TableBody>
                </Table>
              </div>
            ) : (
              <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                <Server className="h-12 w-12 mb-4" />
                <h3 className="text-lg font-medium mb-2">{ui("No Workers Connected")}</h3>
                <p className="text-sm text-center max-w-md">
                  {ui(
                    "No workers have registered with the system. Start a worker process to begin processing tasks.",
                  )}
                </p>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </PageContainer>
  );
}
