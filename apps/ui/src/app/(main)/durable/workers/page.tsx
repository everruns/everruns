"use client";

import { Header } from "@/components/layout/header";
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
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useWorkers, useDrainWorker } from "@/hooks";
import type { DurableWorker, WorkerStatus } from "@/lib/api/types";
import {
  Server,
  AlertTriangle,
  Activity,
  Clock,
  RefreshCw,
  Pause,
  CheckCircle,
} from "lucide-react";

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

function getStatusColor(status: WorkerStatus) {
  switch (status) {
    case "active":
      return "bg-green-500";
    case "draining":
      return "bg-yellow-500";
    case "stopped":
      return "bg-red-500";
    case "stale":
      return "bg-gray-500";
    default:
      return "bg-gray-500";
  }
}

function getStatusBadgeVariant(status: WorkerStatus) {
  switch (status) {
    case "active":
      return "default" as const;
    case "draining":
      return "secondary" as const;
    case "stopped":
      return "destructive" as const;
    case "stale":
      return "outline" as const;
    default:
      return "outline" as const;
  }
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms.toFixed(0)}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${(ms / 60000).toFixed(1)}m`;
}

function WorkerRow({ worker, onDrain }: { worker: DurableWorker; onDrain: (id: string) => void }) {
  const loadPercentage = worker.max_concurrency > 0
    ? (worker.current_load / worker.max_concurrency) * 100
    : 0;

  return (
    <TableRow>
      <TableCell>
        <div className="flex items-center gap-2">
          <div className={`w-2 h-2 rounded-full ${getStatusColor(worker.status)}`} />
          <div>
            <p className="font-medium">{worker.hostname || worker.id.slice(0, 20)}</p>
            <p className="text-xs text-muted-foreground font-mono">{worker.id.slice(0, 16)}...</p>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant={getStatusBadgeVariant(worker.status)}>{worker.status}</Badge>
      </TableCell>
      <TableCell>
        <Badge variant="outline">{worker.worker_group}</Badge>
      </TableCell>
      <TableCell>
        <div className="space-y-1">
          <div className="flex items-center justify-between text-sm">
            <span>{worker.current_load}/{worker.max_concurrency}</span>
            <span className="text-muted-foreground text-xs">{loadPercentage.toFixed(0)}%</span>
          </div>
          <div className="h-1.5 bg-muted rounded-full overflow-hidden w-20">
            <div
              className={`h-full transition-all ${
                loadPercentage > 80 ? "bg-red-500" :
                loadPercentage > 60 ? "bg-yellow-500" : "bg-green-500"
              }`}
              style={{ width: `${loadPercentage}%` }}
            />
          </div>
        </div>
      </TableCell>
      <TableCell>
        {worker.accepting_tasks ? (
          <div className="flex items-center gap-1 text-green-600">
            <CheckCircle className="h-3 w-3" />
            <span className="text-xs">Yes</span>
          </div>
        ) : (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger>
                <div className="flex items-center gap-1 text-yellow-600">
                  <Pause className="h-3 w-3" />
                  <span className="text-xs">No</span>
                </div>
              </TooltipTrigger>
              <TooltipContent>
                {worker.backpressure_reason || "Under backpressure"}
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
          <p className="text-green-600">{worker.tasks_completed.toLocaleString()}</p>
          {worker.tasks_failed > 0 && (
            <p className="text-xs text-red-600">{worker.tasks_failed} failed</p>
          )}
        </div>
      </TableCell>
      <TableCell>
        <span className="text-sm">{formatDuration(worker.avg_task_duration_ms)}</span>
      </TableCell>
      <TableCell>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger className="text-sm text-muted-foreground">
              {formatDistanceToNow(new Date(worker.last_heartbeat_at), { addSuffix: true })}
            </TooltipTrigger>
            <TooltipContent>
              {new Date(worker.last_heartbeat_at).toLocaleString()}
            </TooltipContent>
          </Tooltip>
        </TooltipProvider>
      </TableCell>
      <TableCell>
        {worker.status === "active" && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => onDrain(worker.id)}
          >
            <Pause className="h-3 w-3 mr-1" />
            Drain
          </Button>
        )}
      </TableCell>
    </TableRow>
  );
}

export default function WorkersPage() {
  const { data, isLoading, error, refetch } = useWorkers();
  const drainMutation = useDrainWorker();

  if (isLoading) {
    return (
      <>
        <Header title="Workers" />
        <div className="p-6 space-y-6">
          <div className="grid gap-4 md:grid-cols-4">
            {[...Array(4)].map((_, i) => (
              <Skeleton key={i} className="h-24" />
            ))}
          </div>
          <Skeleton className="h-96" />
        </div>
      </>
    );
  }

  if (error) {
    return (
      <>
        <Header title="Workers" />
        <div className="p-6">
          <Card>
            <CardContent className="flex flex-col items-center justify-center py-12">
              <AlertTriangle className="h-12 w-12 text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">Unable to Load Workers</h3>
              <p className="text-sm text-muted-foreground text-center max-w-md mb-4">
                The durable workers API is not available. Please ensure the backend is running.
              </p>
              <Button onClick={() => refetch()} variant="outline">
                <RefreshCw className="h-4 w-4 mr-2" />
                Retry
              </Button>
            </CardContent>
          </Card>
        </div>
      </>
    );
  }

  const summary = data?.summary;
  const workers = data?.workers || [];

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

  const handleDrain = (workerId: string) => {
    if (confirm(`Are you sure you want to drain worker ${workerId.slice(0, 12)}...?`)) {
      drainMutation.mutate(workerId);
    }
  };

  return (
    <>
      <Header title="Workers" />
      <div className="p-6 space-y-6">
        {/* Summary Stats */}
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
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
          <CardHeader className="flex flex-row items-center justify-between">
            <div>
              <CardTitle>Worker Pool</CardTitle>
              <CardDescription>
                All registered workers and their current status
              </CardDescription>
            </div>
            <Button variant="outline" size="sm" onClick={() => refetch()}>
              <RefreshCw className="h-4 w-4 mr-2" />
              Refresh
            </Button>
          </CardHeader>
          <CardContent>
            {workers.length > 0 ? (
              <div className="rounded-md border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Worker</TableHead>
                      <TableHead>Status</TableHead>
                      <TableHead>Group</TableHead>
                      <TableHead>Load</TableHead>
                      <TableHead>Accepting</TableHead>
                      <TableHead>Activities</TableHead>
                      <TableHead>Completed</TableHead>
                      <TableHead>Avg Duration</TableHead>
                      <TableHead>Last Heartbeat</TableHead>
                      <TableHead>Actions</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {workers.map((worker) => (
                      <WorkerRow
                        key={worker.id}
                        worker={worker}
                        onDrain={handleDrain}
                      />
                    ))}
                  </TableBody>
                </Table>
              </div>
            ) : (
              <div className="flex flex-col items-center justify-center py-12 text-muted-foreground">
                <Server className="h-12 w-12 mb-4" />
                <h3 className="text-lg font-medium mb-2">No Workers Connected</h3>
                <p className="text-sm text-center max-w-md">
                  No workers have registered with the system. Start a worker process to begin processing tasks.
                </p>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </>
  );
}
